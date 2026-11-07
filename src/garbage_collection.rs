//! Garbage collection (D45): LLVM's strategies, the intrinsics they use, and the order their passes run in.
//!
//! A precise, moving collector has to know which stack slots hold pointers. There are two ways LLVM says so. A strategy built
//! on statepoints, such as `statepoint-example` or `coreclr`, keeps its pointers in an address space LLVM can tell apart, and
//! [`statepoint_pipeline`] or [`Builder::garbage_collection_statepoint`] make each call say what is live across it, for the
//! stack map. A strategy built on roots, such as `shadow-stack`, `ocaml` or `erlang`, names stack slots with
//! [`Builder::garbage_collection_root`], and the program reads them again after anything that can collect.
//!
//! A pointer has to be a pointer in the strategy's address space. An integer never reaches a stack map, so nothing rewrites it
//! when its object moves. And a stack map only covers the compiled program's stack: a runtime's own temporaries need roots of
//! their own.

use crate::bad_call::bad_call;
use crate::basic_block::BasicBlock;
use crate::builder::{Builder, OperandBundle, PointerOperand, set_name};
use crate::enums::TypeKind;
use crate::raw;
use crate::shim;
use crate::text::c_string;
use crate::types::{Dynamic, Type, length_as_u32};
use crate::values::{GarbageCollectionStatepoint, LandingPadInstruction, Value, ValueMarker};
use std::any::TypeId;
use std::ffi::{c_int, c_void};
use std::ops::BitOr;
use std::sync::Mutex;

/// `llvm.experimental.gc.statepoint`, which every statepoint call is.
const STATEPOINT: &str = "llvm.experimental.gc.statepoint";
/// `llvm.experimental.gc.result`.
const RESULT: &str = "llvm.experimental.gc.result";
/// `llvm.experimental.gc.relocate`.
const RELOCATE: &str = "llvm.experimental.gc.relocate";
/// `llvm.gcroot`.
const ROOT: &str = "llvm.gcroot";
/// `llvm.gcread`.
const READ: &str = "llvm.gcread";
/// `llvm.gcwrite`.
const WRITE: &str = "llvm.gcwrite";

/// Where a `gc.statepoint`'s third argument, the callee, is in LLVM's attribute numbering, which starts parameters at 1.
const CALLEE_ATTRIBUTE_INDEX: u32 = 3;
/// How many of a `gc.statepoint`'s arguments come before the ones for the call it wraps.
const ARGUMENTS_BEFORE_THE_CALL: u32 = 5;

// trace: D45
/// Whether LLVM has a strategy of this name, built in or registered.
#[must_use]
pub fn strategy_is_registered(name: &str) -> bool {
    // SAFETY: the shim reads `name.len()` bytes.
    unsafe { shim::lyftar_strategy_is_registered(name.as_ptr().cast(), name.len()) }
}

/// A garbage collection strategy written in Rust, built on statepoints. Give it to [`register_strategy`], then name it with
/// [`set_garbage_collector`](crate::values::Value::set_garbage_collector).
///
/// LLVM's `place-safepoints` only knows `statepoint-example` and `coreclr` by name, so with one of these the program has to make
/// its own polls. Strategies built on roots need an emitter of their tables, which is C++, so they aren't offered here.
pub trait GarbageCollectionStrategy: Send + Sync + 'static {
    /// What a function names it by.
    const NAME: &'static str;
    /// What LLVM shows when it lists strategies.
    const DESCRIPTION: &'static str;
    /// Whether `rewrite-statepoints-for-gc` works on functions using this. Without it, the program writes its own statepoints.
    const REWRITES_STATEPOINTS: bool = true;

    /// Whether a pointer of this type is one the collector manages, or `None` if this strategy can't tell, in which case LLVM
    /// treats every pointer as managed. A panic here aborts, since it is running inside LLVM (D3).
    fn manages_pointer(&self, pointer_type: Type<'_>) -> Option<bool>;
}

/// Which strategies this crate registered, by the type that did it.
static REGISTERED: Mutex<Vec<(&str, TypeId)>> = Mutex::new(Vec::new());

// trace: D45
/// Registers a strategy for the rest of this process. In a plugin, call this from what `pass_plugin!` registers. Calling it again
/// with the same type does nothing.
///
/// # Panics
/// If its name has a NUL byte in it, or another strategy has that name (D3).
#[track_caller]
pub fn register_strategy<S: GarbageCollectionStrategy>(strategy: S) {
    unsafe extern "C" fn manages<S: GarbageCollectionStrategy>(
        state: *const c_void,
        pointer_type: raw::LLVMTypeRef,
    ) -> c_int {
        let answer = std::panic::catch_unwind(|| {
            // SAFETY: `state` is the leaked strategy below, for good, and LLVM gives a live type.
            let (strategy, pointer_type) = unsafe { (&*state.cast::<S>(), Type::from_raw(pointer_type)) };
            strategy.manages_pointer(pointer_type)
        });
        match answer {
            Ok(None) => -1,
            Ok(Some(answer)) => c_int::from(answer),
            Err(_) => std::process::abort(),
        }
    }
    let operation = "register_strategy";
    drop(c_string(S::NAME, operation));
    crate::target::set_up_targets();
    // A poisoned list only means another registration panicked, which left the list as it was.
    let mut registered = REGISTERED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let type_id = TypeId::of::<S>();
    if registered.iter().any(|(name, found)| *name == S::NAME && *found == type_id) {
        return;
    }
    if strategy_is_registered(S::NAME) {
        bad_call(format_args!("{operation}: another strategy is called \"{}\"", S::NAME));
    }
    let state: *const S = Box::leak(Box::new(strategy));
    // SAFETY: the lock is held and the name is free, the strategy is leaked so it lasts for good, and `manages` doesn't panic across.
    unsafe {
        shim::lyftar_strategy_register(
            S::NAME.as_ptr().cast(),
            S::NAME.len(),
            S::DESCRIPTION.as_ptr().cast(),
            S::DESCRIPTION.len(),
            S::REWRITES_STATEPOINTS,
            manages::<S>,
            state.cast(),
        );
    }
    registered.push((S::NAME, type_id));
}

/// The flags a statepoint takes, which can be combined with `|`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct StatepointFlags(u32);

impl StatepointFlags {
    /// No flags.
    pub const NONE: Self = StatepointFlags(0);
    /// A transition from code that knows about garbage collection to code that doesn't, or back.
    pub const TRANSITION: Self = StatepointFlags(1);
    /// The deoptimization arguments only need to be there as the call starts, so unused argument registers can hold them.
    pub const DEOPTIMIZATION_LIVE_IN: Self = StatepointFlags(2);

    /// LLVM's number for these flags.
    #[must_use]
    pub fn bits(self) -> u32 {
        self.0
    }
}

impl BitOr for StatepointFlags {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        StatepointFlags(self.0 | other.0)
    }
}

/// What a `gc.statepoint` is made of, for [`Builder::garbage_collection_statepoint`].
#[derive(Clone, Copy, Debug)]
pub struct StatepointCall<'call, 'context, 'module> {
    /// The number the stack map record carries.
    pub id: u64,
    /// How many bytes of no-ops to leave in place of the call, for patching it later. With 0, the call stays.
    pub patch_size_in_bytes: u32,
    /// The type of the function being called.
    pub function_type: Type<'context>,
    /// The function being called.
    pub callee: Value<'context, 'module>,
    /// The call's flags.
    pub flags: StatepointFlags,
    /// The call's arguments.
    pub arguments: &'call [Value<'context, 'module>],
    /// The `gc-transition` bundle.
    pub transition: &'call [Value<'context, 'module>],
    /// The `deopt` bundle.
    pub deoptimization: &'call [Value<'context, 'module>],
    /// The `gc-live` bundle: the pointers that are live across the call, each one a pointer the strategy manages, or a stack
    /// allocation holding them.
    pub live: &'call [Value<'context, 'module>],
}

/// Whether [`statepoint_pipeline`] places safepoints, which needs a `gc.safepoint_poll` function in the module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SafepointPlacement {
    /// Only what the program already calls can collect.
    CallsOnly,
    /// `place-safepoints` adds polls at function entries and loop backedges too, for `statepoint-example` and `coreclr`.
    PlacePolls,
}

// trace: D45
/// Pipeline text that runs in the order statepoints need: the optimizations given, which include any inlining, then any
/// safepoint placement, then `rewrite-statepoints-for-gc`. A rewrite before inlining puts safepoints into code that then moves.
///
/// `optimizations` is pipeline text such as `default<O2>`, or empty for none.
#[must_use]
pub fn statepoint_pipeline(optimizations: &str, placement: SafepointPlacement) -> String {
    let mut pipeline = String::from(optimizations);
    if !pipeline.is_empty() {
        pipeline.push(',');
    }
    if placement == SafepointPlacement::PlacePolls {
        pipeline.push_str("function(place-safepoints),");
    }
    pipeline.push_str("rewrite-statepoints-for-gc");
    pipeline
}

/// A vector's element type, and any other type as it is.
fn scalar_type(value_type: Type<'_>) -> Type<'_> {
    match value_type.kind() {
        TypeKind::Vector | TypeKind::ScalableVector => value_type.element_type(),
        _ => value_type,
    }
}

/// Whether a pointer this function's strategy says it doesn't manage, which is a request the stack map can't honour.
fn strategy_refuses(function: raw::LLVMValueRef, value: Value<'_, '_>) -> bool {
    let pointer_type = scalar_type(value.get_type());
    // SAFETY: `set_garbage_collector` and the verifier keep a function's strategy registered, and this is a pointer type.
    pointer_type.kind() == TypeKind::Pointer
        && unsafe { shim::lyftar_strategy_manages_pointer(function, pointer_type.raw) } == 0
}

fn intrinsic_id(name: &str) -> u32 {
    // SAFETY: LLVM reads `name.len()` bytes.
    unsafe { raw::LLVMLookupIntrinsicID(name.as_ptr().cast(), name.len()) }
}

impl<'context, 'module> Builder<'context, 'module> {
    #[track_caller]
    fn check_garbage_collector(&self, operation: &str) -> raw::LLVMValueRef {
        let function = self.function.get();
        // SAFETY: the builder's function is live, and LLVM's name is only read here.
        let name = unsafe { raw::LLVMGetGC(function) };
        if name.is_null() {
            bad_call(format_args!(
                "{operation}: this function has no garbage collector: call set_garbage_collector on it first"
            ));
        }
        function
    }

    fn intrinsic_declaration(&self, id: u32, overloads: &[raw::LLVMTypeRef]) -> (raw::LLVMValueRef, raw::LLVMTypeRef) {
        // SAFETY: each caller gives what this intrinsic is overloaded on, and the module is live.
        unsafe {
            let declaration =
                raw::LLVMGetIntrinsicDeclaration(self.module, id, overloads.as_ptr().cast_mut(), overloads.len());
            (declaration, raw::LLVMGlobalGetValueType(declaration))
        }
    }

    #[track_caller]
    fn check_statepoint(
        &self,
        operation: &str,
        call: &StatepointCall<'_, 'context, 'module>,
    ) -> (raw::LLVMValueRef, raw::LLVMTypeRef, Vec<raw::LLVMValueRef>, Vec<OperandBundle<'context, 'module>>) {
        let function = self.check_garbage_collector(operation);
        self.check_module(operation, call.callee);
        <Dynamic as PointerOperand>::check_pointer(operation, call.callee);
        if !crate::checks::statepoint_patch_size_is_valid(call.patch_size_in_bytes) {
            bad_call(format_args!(
                "{operation}: LLVM keeps {} bytes of patch space as a positive 32-bit number, and this doesn't fit",
                call.patch_size_in_bytes
            ));
        }
        let count = self.check_call(operation, call.function_type, call.arguments);
        if call.function_type.is_variadic() && call.function_type.return_type().kind() != TypeKind::Void {
            bad_call(format_args!("{operation}: LLVM can only wrap a variadic function that returns void"));
        }
        self.check_values_module(operation, call.transition);
        self.check_values_module(operation, call.deoptimization);
        self.check_values_module(operation, call.live);
        for value in call.live {
            // SAFETY: only reads it.
            let is_allocation = unsafe { shim::lyftar_value_is_stack_allocation(value.raw) };
            let scalar = scalar_type(value.get_type());
            if !is_allocation && scalar.kind() != TypeKind::Pointer {
                bad_call(format_args!(
                    "{operation}: {} isn't a pointer, and only a pointer reaches the stack map, so nothing would rewrite it",
                    value.describe()
                ));
            }
            if !is_allocation && strategy_refuses(function, *value) {
                bad_call(format_args!(
                    "{operation}: {} is a pointer this function's garbage collector doesn't manage, so nothing would rewrite it",
                    value.describe()
                ));
            }
        }
        // SAFETY: the module is live, so its context is too, and these make plain constants.
        let (arguments, overload) = unsafe {
            let context = raw::LLVMGetModuleContext(self.module);
            let i32_type = raw::LLVMInt32TypeInContext(context);
            let i64_type = raw::LLVMInt64TypeInContext(context);
            let zero = raw::LLVMConstInt(i32_type, 0, 0);
            let mut arguments = Vec::with_capacity(call.arguments.len() + 7);
            arguments.push(raw::LLVMConstInt(i64_type, call.id, 0));
            arguments.push(raw::LLVMConstInt(i32_type, u64::from(call.patch_size_in_bytes), 0));
            arguments.push(call.callee.raw);
            arguments.push(raw::LLVMConstInt(i32_type, u64::from(count), 0));
            arguments.push(raw::LLVMConstInt(i32_type, u64::from(call.flags.bits()), 0));
            arguments.extend(call.arguments.iter().map(|argument| argument.raw));
            // Transition and deoptimization arguments only go in bundles now, and LLVM still wants these two counts.
            arguments.push(zero);
            arguments.push(zero);
            (arguments, call.callee.get_type().raw)
        };
        let (declaration, intrinsic_type) = self.intrinsic_declaration(intrinsic_id(STATEPOINT), &[overload]);
        let mut bundles = Vec::with_capacity(3);
        if !call.deoptimization.is_empty() {
            bundles.push(OperandBundle::new("deopt", call.deoptimization));
        }
        if !call.transition.is_empty() {
            bundles.push(OperandBundle::new("gc-transition", call.transition));
        }
        if !call.live.is_empty() {
            bundles.push(OperandBundle::new("gc-live", call.live));
        }
        (declaration, intrinsic_type, arguments, bundles)
    }

    fn name_statepoint(
        &self,
        statepoint: raw::LLVMValueRef,
        function_type: Type<'context>,
        name: &str,
    ) -> Value<'context, 'module, GarbageCollectionStatepoint> {
        // SAFETY: LLVM just built this call, whose third argument is the callee, and it names this attribute.
        unsafe {
            let context = raw::LLVMGetModuleContext(self.module);
            let kind = raw::LLVMGetEnumAttributeKindForName(c"elementtype".as_ptr(), "elementtype".len());
            let attribute = raw::LLVMCreateTypeAttribute(context, kind, function_type.raw);
            raw::LLVMAddCallSiteAttribute(statepoint, CALLEE_ATTRIBUTE_INDEX, attribute);
        }
        set_name(statepoint, name);
        // SAFETY: this is a `gc.statepoint` LLVM just built, live for `'module`.
        unsafe { Value::from_raw(statepoint) }
    }

    /// Calls a function with a `gc.statepoint`, which says what stays live across it and has the stack map record it. Use each
    /// live pointer only through its [`garbage_collection_relocate`](Self::garbage_collection_relocate) afterwards, since the
    /// old value still points where the object was.
    ///
    /// # Panics
    /// If this function has no garbage collector, the arguments don't fit the function type, something in `live` isn't a
    /// pointer its strategy manages or a stack allocation, a value belongs to another module, or the block is already
    /// terminated (D3).
    #[track_caller]
    pub fn garbage_collection_statepoint(
        &self,
        call: &StatepointCall<'_, 'context, 'module>,
        name: &str,
    ) -> Value<'context, 'module, GarbageCollectionStatepoint> {
        self.check_open("garbage_collection_statepoint");
        let (declaration, intrinsic_type, arguments, bundles) =
            self.check_statepoint("garbage_collection_statepoint", call);
        // SAFETY: the checks above are what LLVM requires, and it only reads the lists. `OperandBundle` has the layout of its
        // raw pointer.
        let statepoint = unsafe {
            raw::LLVMBuildCallWithOperandBundles(
                self.raw,
                intrinsic_type,
                declaration,
                arguments.as_ptr().cast_mut(),
                length_as_u32(arguments.len(), "garbage_collection_statepoint"),
                bundles.as_ptr().cast_mut().cast(),
                length_as_u32(bundles.len(), "garbage_collection_statepoint"),
                c"".as_ptr(),
            )
        };
        self.name_statepoint(statepoint, call.function_type, name)
    }

    /// The form of [`garbage_collection_statepoint`](Self::garbage_collection_statepoint) that can unwind: an `invoke`. Its
    /// unwind destination reads relocated pointers with
    /// [`garbage_collection_relocate_on_unwind`](Self::garbage_collection_relocate_on_unwind).
    ///
    /// # Panics
    /// As for [`garbage_collection_statepoint`](Self::garbage_collection_statepoint), and if a block is in another function (D3).
    #[track_caller]
    pub fn garbage_collection_statepoint_invoke(
        &self,
        call: &StatepointCall<'_, 'context, 'module>,
        normal_destination: BasicBlock<'context, 'module>,
        unwind_destination: BasicBlock<'context, 'module>,
        name: &str,
    ) -> Value<'context, 'module, GarbageCollectionStatepoint> {
        self.check_terminator_position("garbage_collection_statepoint_invoke");
        self.check_destination("garbage_collection_statepoint_invoke", normal_destination);
        self.check_destination("garbage_collection_statepoint_invoke", unwind_destination);
        let (declaration, intrinsic_type, arguments, bundles) =
            self.check_statepoint("garbage_collection_statepoint_invoke", call);
        // SAFETY: as for a call, and both blocks were just checked.
        let statepoint = unsafe {
            raw::LLVMBuildInvokeWithOperandBundles(
                self.raw,
                intrinsic_type,
                declaration,
                arguments.as_ptr().cast_mut(),
                length_as_u32(arguments.len(), "garbage_collection_statepoint_invoke"),
                normal_destination.raw,
                unwind_destination.raw,
                bundles.as_ptr().cast_mut().cast(),
                length_as_u32(bundles.len(), "garbage_collection_statepoint_invoke"),
                c"".as_ptr(),
            )
        };
        self.name_statepoint(statepoint, call.function_type, name)
    }

    /// What the call a statepoint wraps returned: `gc.result`.
    ///
    /// # Panics
    /// If that call returns nothing, the statepoint belongs to another module, this function has no garbage collector, or the
    /// block is already terminated (D3).
    #[track_caller]
    pub fn garbage_collection_result(
        &self,
        statepoint: Value<'context, 'module, GarbageCollectionStatepoint>,
        name: &str,
    ) -> Value<'context, 'module> {
        self.check_open("garbage_collection_result");
        self.check_garbage_collector("garbage_collection_result");
        self.check_module("garbage_collection_result", statepoint);
        let return_type = statepoint.function_type().return_type();
        if return_type.kind() == TypeKind::Void {
            bad_call(format_args!("garbage_collection_result: the call this statepoint wraps returns nothing"));
        }
        let (declaration, intrinsic_type) = self.intrinsic_declaration(intrinsic_id(RESULT), &[return_type.raw]);
        let mut arguments = [statepoint.raw];
        // SAFETY: the type is what the verifier requires, and LLVM only reads the argument.
        let result = unsafe {
            raw::LLVMBuildCall2(self.raw, intrinsic_type, declaration, arguments.as_mut_ptr(), 1, c"".as_ptr())
        };
        set_name(result, name);
        // SAFETY: LLVM just built it, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    #[track_caller]
    fn build_relocate(
        &self,
        operation: &str,
        statepoint: Value<'context, 'module, GarbageCollectionStatepoint>,
        token: raw::LLVMValueRef,
        base_index: u32,
        derived_index: u32,
        name: &str,
    ) -> Value<'context, 'module> {
        let function = self.check_garbage_collector(operation);
        let live = statepoint.live();
        let count = length_as_u32(live.len(), operation);
        for index in [base_index, derived_index] {
            if !crate::checks::index_in_bounds(index, count) {
                bad_call(format_args!("{operation}: index {index} is past this statepoint's {count} live values"));
            }
        }
        let base = live[base_index as usize];
        let derived = live[derived_index as usize];
        for value in [base, derived] {
            if scalar_type(value.get_type()).kind() != TypeKind::Pointer || strategy_refuses(function, value) {
                bad_call(format_args!(
                    "{operation}: {} is not a pointer this function's garbage collector manages, so it has nothing to relocate",
                    value.describe()
                ));
            }
        }
        let result_type = derived.get_type();
        let (declaration, intrinsic_type) = self.intrinsic_declaration(intrinsic_id(RELOCATE), &[result_type.raw]);
        // SAFETY: the module is live, so its context is too, and the indices were just checked.
        let result = unsafe {
            let i32_type = raw::LLVMInt32TypeInContext(raw::LLVMGetModuleContext(self.module));
            let mut arguments = [
                token,
                raw::LLVMConstInt(i32_type, u64::from(base_index), 0),
                raw::LLVMConstInt(i32_type, u64::from(derived_index), 0),
            ];
            raw::LLVMBuildCall2(self.raw, intrinsic_type, declaration, arguments.as_mut_ptr(), 3, c"".as_ptr())
        };
        set_name(result, name);
        // SAFETY: LLVM just built it, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Where a pointer in a statepoint's `live` list is now: `gc.relocate`. The indices point into that list, and the result
    /// has the type of the derived pointer.
    ///
    /// # Panics
    /// If an index is out of bounds, what it names isn't a pointer the strategy manages, the statepoint belongs to another
    /// module, this function has no garbage collector, or the block is already terminated (D3).
    #[track_caller]
    pub fn garbage_collection_relocate(
        &self,
        statepoint: Value<'context, 'module, GarbageCollectionStatepoint>,
        base_index: u32,
        derived_index: u32,
        name: &str,
    ) -> Value<'context, 'module> {
        self.check_open("garbage_collection_relocate");
        self.check_module("garbage_collection_relocate", statepoint);
        self.build_relocate("garbage_collection_relocate", statepoint, statepoint.raw, base_index, derived_index, name)
    }

    /// [`garbage_collection_relocate`](Self::garbage_collection_relocate) on the unwinding path of an invoke, where it reads
    /// from the landing pad.
    ///
    /// # Panics
    /// As for [`garbage_collection_relocate`](Self::garbage_collection_relocate), and if the landing pad's block doesn't have
    /// exactly one predecessor, ending in a statepoint (D3).
    #[track_caller]
    pub fn garbage_collection_relocate_on_unwind(
        &self,
        landing_pad: Value<'context, 'module, LandingPadInstruction>,
        base_index: u32,
        derived_index: u32,
        name: &str,
    ) -> Value<'context, 'module> {
        let operation = "garbage_collection_relocate_on_unwind";
        self.check_open(operation);
        self.check_module(operation, landing_pad);
        if landing_pad.get_type().kind() != TypeKind::Token {
            bad_call(format_args!(
                "{operation}: a gc.relocate reads a landing pad of type `token`, not {}",
                landing_pad.describe()
            ));
        }
        // SAFETY: a landing pad is an instruction, in a live block.
        let statepoint = unsafe {
            let block = raw::LLVMGetInstructionParent(landing_pad.raw);
            let predecessor = shim::lyftar_block_unique_predecessor(block);
            if predecessor.is_null() {
                bad_call(format_args!("{operation}: a statepoint's landing pad has to have exactly one predecessor"));
            }
            raw::LLVMGetBasicBlockTerminator(predecessor)
        };
        // SAFETY: LLVM gave this back, or null for a block with no terminator, which `accepts_value` isn't asked about.
        if statepoint.is_null() || !unsafe { GarbageCollectionStatepoint::accepts_value(statepoint) } {
            bad_call(format_args!("{operation}: this landing pad's predecessor doesn't end in a statepoint"));
        }
        // SAFETY: just checked.
        let statepoint = unsafe { Value::from_raw(statepoint) };
        self.build_relocate(operation, statepoint, landing_pad.raw, base_index, derived_index, name)
    }

    /// Marks a stack slot as holding a pointer the collector can rewrite: `llvm.gcroot`. `metadata` is constant data the strategy
    /// keeps with the root, or a null pointer for none. The program has to read the slot again after anything that can collect.
    ///
    /// # Panics
    /// If this function has no garbage collector, `slot` isn't a fixed-size stack allocation in the entry block, `metadata` isn't
    /// a constant, the slot holds something other than a pointer and `metadata` is null, a value belongs to another module, or
    /// the block is already terminated (D3).
    #[track_caller]
    pub fn garbage_collection_root<P: PointerOperand, M: PointerOperand>(
        &self,
        slot: Value<'context, 'module, P>,
        metadata: Value<'context, 'module, M>,
    ) {
        let operation = "garbage_collection_root";
        self.check_open(operation);
        self.check_garbage_collector(operation);
        self.check_module(operation, slot);
        self.check_module(operation, metadata);
        // SAFETY: only reads it.
        if !unsafe { shim::lyftar_value_is_static_stack_allocation(slot.raw) } {
            bad_call(format_args!(
                "{operation}: {} isn't a fixed-size stack allocation in the entry block, the only kind that gets a stack slot",
                slot.describe()
            ));
        }
        // SAFETY: both are live values, and these only read them.
        unsafe {
            if raw::LLVMIsAConstant(metadata.raw).is_null() {
                bad_call(format_args!("{operation}: {} isn't a constant", metadata.describe()));
            }
            if !shim::lyftar_stack_allocation_holds_a_pointer(slot.raw) && raw::LLVMIsNull(metadata.raw) != 0 {
                bad_call(format_args!(
                    "{operation}: {} holds something other than a pointer, and that needs metadata that says what it is",
                    slot.describe()
                ));
            }
        }
        self.call_memory_intrinsic(operation, ROOT, &[slot.to_dynamic(), metadata.to_dynamic()]);
    }

    /// Reads a pointer out of an object through the strategy's read barrier: `llvm.gcread`. `object` is the object and
    /// `location` the field in it.
    ///
    /// # Panics
    /// If this function has no garbage collector, the pointers aren't in address space 0, a value belongs to another module, or
    /// the block is already terminated (D3).
    #[track_caller]
    pub fn garbage_collection_read<O: PointerOperand, L: PointerOperand>(
        &self,
        object: Value<'context, 'module, O>,
        location: Value<'context, 'module, L>,
        name: &str,
    ) -> Value<'context, 'module, crate::types::Pointer> {
        let operation = "garbage_collection_read";
        self.check_open(operation);
        self.check_garbage_collector(operation);
        let result = self.call_memory_intrinsic(operation, READ, &[object.to_dynamic(), location.to_dynamic()]);
        set_name(result, name);
        // SAFETY: `llvm.gcread` returns a pointer in address space 0, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Writes a pointer into an object through the strategy's write barrier: `llvm.gcwrite`. `value` goes into the field
    /// `location` of `object`.
    ///
    /// # Panics
    /// If this function has no garbage collector, the pointers aren't in address space 0, a value belongs to another module, or
    /// the block is already terminated (D3).
    #[track_caller]
    pub fn garbage_collection_write<V: PointerOperand, O: PointerOperand, L: PointerOperand>(
        &self,
        value: Value<'context, 'module, V>,
        object: Value<'context, 'module, O>,
        location: Value<'context, 'module, L>,
    ) {
        let operation = "garbage_collection_write";
        self.check_open(operation);
        self.check_garbage_collector(operation);
        self.call_memory_intrinsic(operation, WRITE, &[value.to_dynamic(), object.to_dynamic(), location.to_dynamic()]);
    }

    /// Calls one of `llvm.gcroot`, `llvm.gcread` and `llvm.gcwrite`, which aren't overloaded and take plain pointers.
    #[track_caller]
    fn call_memory_intrinsic(
        &self,
        operation: &str,
        intrinsic: &str,
        arguments: &[Value<'context, 'module>],
    ) -> raw::LLVMValueRef {
        let (declaration, intrinsic_type) = self.intrinsic_declaration(intrinsic_id(intrinsic), &[]);
        // SAFETY: LLVM just gave this type.
        let function_type: Type<'context> = unsafe { Type::from_raw(intrinsic_type) };
        let count = self.check_call(operation, function_type, arguments);
        // SAFETY: the checks above are what LLVM requires, and it only reads the arguments.
        unsafe {
            raw::LLVMBuildCall2(
                self.raw,
                intrinsic_type,
                declaration,
                arguments.as_ptr().cast_mut().cast(),
                count,
                c"".as_ptr(),
            )
        }
    }
}

impl<'context, 'module> Value<'context, 'module, GarbageCollectionStatepoint> {
    /// The type of the function this statepoint calls.
    #[must_use]
    pub fn function_type(self) -> Type<'context> {
        // SAFETY: the marker guarantees a `gc.statepoint`, and the verifier and this crate's builder give its callee the
        // attribute. It says the type lives as long as the context.
        unsafe {
            let kind = raw::LLVMGetEnumAttributeKindForName(c"elementtype".as_ptr(), "elementtype".len());
            let attribute = raw::LLVMGetCallSiteEnumAttribute(self.raw, CALLEE_ATTRIBUTE_INDEX, kind);
            Type::from_raw(raw::LLVMGetTypeAttributeValue(attribute))
        }
    }

    fn raw_operand(self, index: u32) -> raw::LLVMValueRef {
        // SAFETY: the marker guarantees a `gc.statepoint`, whose first seven operands are always there, and each caller's index
        // is below that or below the argument count LLVM gave.
        unsafe { raw::LLVMGetOperand(self.raw, index) }
    }

    fn constant_operand(self, index: u32) -> u64 {
        // SAFETY: the verifier and this crate's builder keep these operands constant integers.
        unsafe { raw::LLVMConstIntGetZExtValue(self.raw_operand(index)) }
    }

    /// The number the stack map record carries.
    #[must_use]
    pub fn id(self) -> u64 {
        self.constant_operand(0)
    }

    /// How many bytes of no-ops take the place of the call, for patching it later.
    #[must_use]
    pub fn patch_size_in_bytes(self) -> u64 {
        self.constant_operand(1)
    }

    /// LLVM's number for the flags.
    #[must_use]
    pub fn flag_bits(self) -> u64 {
        self.constant_operand(4)
    }

    /// The function being called.
    #[must_use]
    pub fn callee(self) -> Value<'context, 'module> {
        // SAFETY: operand 2 is the callee, live as long as this call.
        unsafe { Value::from_raw(self.raw_operand(2)) }
    }

    /// The arguments of the call being wrapped.
    #[must_use]
    pub fn call_arguments(self) -> Vec<Value<'context, 'module>> {
        let count = u32::try_from(self.constant_operand(3)).unwrap_or(0);
        (0..count)
            // SAFETY: the verifier and this crate's builder keep this many arguments here, live as long as this call.
            .map(|index| unsafe { Value::from_raw(self.raw_operand(ARGUMENTS_BEFORE_THE_CALL + index)) })
            .collect()
    }

    fn bundle(self, tag: &str) -> Vec<Value<'context, 'module>> {
        self.operand_bundles().iter().find(|bundle| bundle.tag() == tag).map_or(Vec::new(), OperandBundle::arguments)
    }

    /// What is in the `gc-live` bundle.
    #[must_use]
    pub fn live(self) -> Vec<Value<'context, 'module>> {
        self.bundle("gc-live")
    }

    /// What is in the `deopt` bundle.
    #[must_use]
    pub fn deoptimization(self) -> Vec<Value<'context, 'module>> {
        self.bundle("deopt")
    }

    /// What is in the `gc-transition` bundle.
    #[must_use]
    pub fn transition(self) -> Vec<Value<'context, 'module>> {
        self.bundle("gc-transition")
    }
}
