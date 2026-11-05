//! The instruction builder (D3, D4).
//!
//! A build call returns its value, and a request LLVM would turn into invalid IR panics at the call and names the problem
//! (D3). Types spelled out in the Rust code are checked at `cargo build`, with no check when the line runs (D4, D24).

use std::cell::Cell;
use std::ffi::CStr;
use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::basic_block::BasicBlock;
use crate::checks;
use crate::enums::{AtomicOrdering, AtomicReadModifyWriteOperation, FloatPredicate, IntegerPredicate, Opcode};
use crate::instructions::{GetElementPointerFlags, SynchronizationScope};
use crate::metadata::Metadata;
use crate::raw;
use crate::shim;
use crate::text::c_string;
use crate::types::{
    AddressSpace, Bits, Dynamic, Float, FloatFormat, Integer, Invariant, Pointer, Type, TypeMarker, is_float_kind,
    length_as_u32,
};
use crate::values::{
    AtomicCompareExchangeInstruction, AtomicReadModifyWriteInstruction, CallBranchInstruction, CallInstruction,
    CatchPadInstruction, CatchReturnInstruction, CatchSwitchInstruction, CleanupPadInstruction,
    CleanupReturnInstruction, ConditionalBranchInstruction, FenceInstruction, Function, GlobalAlias, GlobalObject,
    GlobalValue, GlobalVariable, IndirectBranchInstruction, InstructionMarker, InvokeInstruction,
    LandingPadInstruction, PhiNode, ResumeInstruction, ReturnInstruction, StackAllocationInstruction, StoreInstruction,
    SwitchInstruction, UnconditionalBranchInstruction, UnreachableInstruction, Value, ValueMarker,
};

/// Builds instructions. It only exists placed in a block, so an unset position can't happen (D3).
pub struct Builder<'context, 'module> {
    raw: raw::LLVMBuilderRef,
    module: raw::LLVMModuleRef,
    function: Cell<raw::LLVMValueRef>,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl Drop for Builder<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the builder.
        unsafe { raw::LLVMDisposeBuilder(self.raw) };
    }
}

/// A piece of the check for integer operands: an integer or a vector of integers.
fn is_integer_like(raw_type: raw::LLVMTypeRef) -> bool {
    use raw::LLVMTypeKind as K;
    // SAFETY: the type came from a live value.
    unsafe {
        let scalar = match raw::LLVMGetTypeKind(raw_type) {
            K::LLVMVectorTypeKind | K::LLVMScalableVectorTypeKind => raw::LLVMGetElementType(raw_type),
            _ => raw_type,
        };
        raw::LLVMGetTypeKind(scalar) == K::LLVMIntegerTypeKind
    }
}

/// A piece of the check for float operands: a float or a vector of floats.
fn is_float_like(raw_type: raw::LLVMTypeRef) -> bool {
    use raw::LLVMTypeKind as K;
    // SAFETY: the type came from a live value.
    unsafe {
        let scalar = match raw::LLVMGetTypeKind(raw_type) {
            K::LLVMVectorTypeKind | K::LLVMScalableVectorTypeKind => raw::LLVMGetElementType(raw_type),
            _ => raw_type,
        };
        is_float_kind(raw::LLVMGetTypeKind(scalar))
    }
}

#[cold]
#[track_caller]
fn bad_operand(operation: &str, value: raw::LLVMValueRef, wanted: &str) -> ! {
    // SAFETY: the caller got this from a live `Value`.
    let value: Value<'_, '_> = unsafe { Value::from_raw(value) };
    bad_call(format_args!("{operation}: {} is not {wanted}", value.describe()))
}

#[cold]
#[track_caller]
fn different_types(operation: &str, left: raw::LLVMValueRef, right: raw::LLVMValueRef) -> ! {
    // SAFETY: the caller got these from live `Value`s.
    let (left, right): (Value<'_, '_>, Value<'_, '_>) = unsafe { (Value::from_raw(left), Value::from_raw(right)) };
    bad_call(format_args!(
        "{operation}: the operands have different types, {} and {}",
        left.get_type().describe(),
        right.get_type().describe()
    ))
}

// trace: D3
#[inline]
#[track_caller]
fn check_same_type(operation: &str, left: raw::LLVMValueRef, right: raw::LLVMValueRef) {
    // SAFETY: both came from live `Value`s.
    let (left_type, right_type) = unsafe { (raw::LLVMTypeOf(left), raw::LLVMTypeOf(right)) };
    if !checks::same_identity(left_type as usize, right_type as usize) {
        different_types(operation, left, right);
    }
}

/// An integer operand, and what a build call has to check about it when the line runs.
pub trait IntegerOperand: ValueMarker + TypeMarker {
    /// What comparing two of these gives.
    type Comparison: ValueMarker;
    /// The width, when the Rust code spells it out.
    const WIDTH_IN_BITS: Option<u32>;
    #[doc(hidden)]
    fn check_one(operation: &str, value: Value<'_, '_>);
    #[doc(hidden)]
    fn check_pair(operation: &str, left: Value<'_, '_>, right: Value<'_, '_>);
}

impl<const N: u32> IntegerOperand for Integer<Bits<N>> {
    type Comparison = Integer<Bits<1>>;
    const WIDTH_IN_BITS: Option<u32> = Some(N);
    // trace: D24
    // The type is spelled out, so `cargo build` already checked it.
    #[inline(always)]
    fn check_one(_operation: &str, _value: Value<'_, '_>) {}
    #[inline(always)]
    fn check_pair(_operation: &str, _left: Value<'_, '_>, _right: Value<'_, '_>) {}
}

impl IntegerOperand for Integer<Dynamic> {
    type Comparison = Integer<Bits<1>>;
    const WIDTH_IN_BITS: Option<u32> = None;
    #[inline(always)]
    fn check_one(_operation: &str, _value: Value<'_, '_>) {}
    #[inline]
    #[track_caller]
    fn check_pair(operation: &str, left: Value<'_, '_>, right: Value<'_, '_>) {
        check_same_type(operation, left.raw, right.raw);
    }
}

impl IntegerOperand for Dynamic {
    type Comparison = Dynamic;
    const WIDTH_IN_BITS: Option<u32> = None;
    #[track_caller]
    fn check_one(operation: &str, value: Value<'_, '_>) {
        // SAFETY: the value came from a live `Value`.
        if !is_integer_like(unsafe { raw::LLVMTypeOf(value.raw) }) {
            bad_operand(operation, value.raw, "an integer or a vector of integers");
        }
    }
    #[track_caller]
    fn check_pair(operation: &str, left: Value<'_, '_>, right: Value<'_, '_>) {
        check_same_type(operation, left.raw, right.raw);
        <Self as IntegerOperand>::check_one(operation, left);
    }
}

/// A float operand, and what a build call has to check about it when the line runs.
pub trait FloatOperand: ValueMarker + TypeMarker {
    /// What comparing two of these gives.
    type Comparison: ValueMarker;
    /// The size of the format, when the Rust code spells it out.
    const SIZE_IN_BITS: Option<u32>;
    #[doc(hidden)]
    fn check_one(operation: &str, value: Value<'_, '_>);
    #[doc(hidden)]
    fn check_pair(operation: &str, left: Value<'_, '_>, right: Value<'_, '_>);
}

impl<F: FloatFormat> FloatOperand for Float<F> {
    type Comparison = Integer<Bits<1>>;
    const SIZE_IN_BITS: Option<u32> = Some(F::SIZE_IN_BITS);
    // trace: D24
    #[inline(always)]
    fn check_one(_operation: &str, _value: Value<'_, '_>) {}
    #[inline(always)]
    fn check_pair(_operation: &str, _left: Value<'_, '_>, _right: Value<'_, '_>) {}
}

impl FloatOperand for Float<Dynamic> {
    type Comparison = Integer<Bits<1>>;
    const SIZE_IN_BITS: Option<u32> = None;
    #[inline(always)]
    fn check_one(_operation: &str, _value: Value<'_, '_>) {}
    #[inline]
    #[track_caller]
    fn check_pair(operation: &str, left: Value<'_, '_>, right: Value<'_, '_>) {
        check_same_type(operation, left.raw, right.raw);
    }
}

impl FloatOperand for Dynamic {
    type Comparison = Dynamic;
    const SIZE_IN_BITS: Option<u32> = None;
    #[track_caller]
    fn check_one(operation: &str, value: Value<'_, '_>) {
        // SAFETY: the value came from a live `Value`.
        if !is_float_like(unsafe { raw::LLVMTypeOf(value.raw) }) {
            bad_operand(operation, value.raw, "a float or a vector of floats");
        }
    }
    #[track_caller]
    fn check_pair(operation: &str, left: Value<'_, '_>, right: Value<'_, '_>) {
        check_same_type(operation, left.raw, right.raw);
        <Self as FloatOperand>::check_one(operation, left);
    }
}

/// Something that can be used as a pointer.
pub trait PointerOperand: ValueMarker {
    /// What a `getelementptr` over this gives.
    type GetElementPointerResult: ValueMarker;
    #[doc(hidden)]
    const IS_DYNAMIC: bool;
    /// Checks for one pointer.
    #[doc(hidden)]
    fn check_pointer(operation: &str, value: Value<'_, '_>);
    /// Checks for one pointer or a vector of them, as `getelementptr` takes.
    #[doc(hidden)]
    fn check_pointer_or_vector(operation: &str, value: Value<'_, '_>);
}

// trace: D31, D24
impl<const N: u32> PointerOperand for Pointer<AddressSpace<N>> {
    type GetElementPointerResult = Self;
    const IS_DYNAMIC: bool = false;
    #[inline(always)]
    fn check_pointer(_operation: &str, _value: Value<'_, '_>) {}
    #[inline(always)]
    fn check_pointer_or_vector(_operation: &str, _value: Value<'_, '_>) {}
}

/// Declares a marker whose value is always a pointer, in an address space only known when the line runs (D31).
macro_rules! always_a_pointer {
    ($($marker:ty),*) => {
        $(impl PointerOperand for $marker {
            type GetElementPointerResult = Dynamic;
            const IS_DYNAMIC: bool = false;
            // trace: D24
            #[inline(always)]
            fn check_pointer(_operation: &str, _value: Value<'_, '_>) {}
            #[inline(always)]
            fn check_pointer_or_vector(_operation: &str, _value: Value<'_, '_>) {}
        })*
    };
}
always_a_pointer!(Function, GlobalVariable, GlobalAlias, GlobalValue, GlobalObject, StackAllocationInstruction);

impl PointerOperand for Dynamic {
    type GetElementPointerResult = Dynamic;
    const IS_DYNAMIC: bool = true;
    #[track_caller]
    fn check_pointer(operation: &str, value: Value<'_, '_>) {
        // SAFETY: the value came from a live `Value`.
        let is_pointer =
            unsafe { raw::LLVMGetTypeKind(raw::LLVMTypeOf(value.raw)) == raw::LLVMTypeKind::LLVMPointerTypeKind };
        if !is_pointer {
            bad_operand(operation, value.raw, "a pointer");
        }
    }

    #[track_caller]
    fn check_pointer_or_vector(operation: &str, value: Value<'_, '_>) {
        use raw::LLVMTypeKind as K;
        // SAFETY: the value came from a live `Value`.
        let is_pointer = unsafe {
            let value_type = raw::LLVMTypeOf(value.raw);
            let scalar = match raw::LLVMGetTypeKind(value_type) {
                K::LLVMVectorTypeKind | K::LLVMScalableVectorTypeKind => raw::LLVMGetElementType(value_type),
                _ => value_type,
            };
            raw::LLVMGetTypeKind(scalar) == K::LLVMPointerTypeKind
        };
        if !is_pointer {
            bad_operand(operation, value.raw, "a pointer or a vector of pointers");
        }
    }
}

/// A condition: `i1`, checked here unless the Rust code spells it out.
pub trait ConditionOperand: ValueMarker {
    #[doc(hidden)]
    const IS_BOOLEAN: bool;
    #[doc(hidden)]
    fn check_condition(operation: &str, value: Value<'_, '_>);
}

impl ConditionOperand for Integer<Bits<1>> {
    const IS_BOOLEAN: bool = true;
    // trace: D24
    #[inline(always)]
    fn check_condition(_operation: &str, _value: Value<'_, '_>) {}
}

/// Checks that a value is exactly `i1`.
#[track_caller]
fn check_boolean(operation: &str, value: raw::LLVMValueRef) {
    use raw::LLVMTypeKind as K;
    // SAFETY: the value came from a live `Value`.
    let is_boolean = unsafe {
        let value_type = raw::LLVMTypeOf(value);
        raw::LLVMGetTypeKind(value_type) == K::LLVMIntegerTypeKind && raw::LLVMGetIntTypeWidth(value_type) == 1
    };
    if !is_boolean {
        bad_operand(operation, value, "an i1");
    }
}

impl ConditionOperand for Integer<Dynamic> {
    const IS_BOOLEAN: bool = false;
    #[track_caller]
    fn check_condition(operation: &str, value: Value<'_, '_>) {
        check_boolean(operation, value.raw);
    }
}

impl ConditionOperand for Dynamic {
    const IS_BOOLEAN: bool = false;
    #[track_caller]
    fn check_condition(operation: &str, value: Value<'_, '_>) {
        check_boolean(operation, value.raw);
    }
}

/// An operand bundle: a tag and values attached to a call.
#[repr(transparent)]
pub struct OperandBundle<'context, 'module> {
    raw: raw::LLVMOperandBundleRef,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl Drop for OperandBundle<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the bundle.
        unsafe { raw::LLVMDisposeOperandBundle(self.raw) };
    }
}

impl<'context, 'module> OperandBundle<'context, 'module> {
    /// A bundle with this tag and these arguments.
    #[must_use]
    #[track_caller]
    pub fn new<T>(tag: &str, arguments: &[Value<'context, 'module, T>]) -> Self {
        let count = length_as_u32(arguments.len(), "OperandBundle::new");
        // SAFETY: LLVM only reads the tag and arguments; `Value` has the layout of `LLVMValueRef`.
        let raw = unsafe {
            raw::LLVMCreateOperandBundle(tag.as_ptr().cast(), tag.len(), arguments.as_ptr().cast_mut().cast(), count)
        };
        OperandBundle { raw, marker: PhantomData }
    }

    /// Takes ownership of a raw bundle.
    ///
    /// # Safety
    /// `raw` must be a live bundle that nothing else owns, whose values stay live for `'module`.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMOperandBundleRef) -> Self {
        OperandBundle { raw, marker: PhantomData }
    }

    /// The raw bundle.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(&self) -> raw::LLVMOperandBundleRef {
        self.raw
    }

    /// This bundle's tag.
    #[must_use]
    pub fn tag(&self) -> String {
        let mut length = 0;
        // SAFETY: the bundle is live; the text is copied right away.
        let text = unsafe { raw::LLVMGetOperandBundleTag(self.raw, &raw mut length) };
        crate::text::from_pointer_and_length(text, length)
    }

    /// This bundle's arguments.
    #[must_use]
    pub fn arguments(&self) -> Vec<Value<'context, 'module>> {
        // SAFETY: the bundle is live, and each index is below the count LLVM gave.
        unsafe {
            (0..raw::LLVMGetNumOperandBundleArgs(self.raw))
                .map(|index| Value::from_raw(raw::LLVMGetOperandBundleArgAtIndex(self.raw, index)))
                .collect()
        }
    }
}

const EMPTY: &CStr = c"";

// trace: D24
/// Names a built value without allocating: `LLVMSetValueName2` takes a length, where the build calls want a C string.
/// A build call can give back a constant or its operand, which must not be renamed.
#[inline]
fn set_name(value: raw::LLVMValueRef, name: &str) {
    if !name.is_empty() {
        // SAFETY: the value is live, and LLVM reads `name.len()` bytes of `name`.
        unsafe {
            if !raw::LLVMIsAInstruction(value).is_null()
                && raw::LLVMGetTypeKind(raw::LLVMTypeOf(value)) != raw::LLVMTypeKind::LLVMVoidTypeKind
            {
                raw::LLVMSetValueName2(value, name.as_ptr().cast(), name.len());
            }
        }
    }
}

/// Declares build calls that take two integer operands.
macro_rules! integer_binary {
    ($($(#[$meta:meta])* $name:ident = $build:ident;)*) => {
        $(
            $(#[$meta])*
            ///
            /// # Panics
            /// If an operand belongs to another module, the types differ, or the block is already terminated (D3).
            #[track_caller]
            pub fn $name<T: IntegerOperand>(
                &self,
                left: Value<'context, 'module, T>,
                right: Value<'context, 'module, T>,
                name: &str,
            ) -> Value<'context, 'module, T> {
                self.check_open(stringify!($name));
                self.check_module(stringify!($name), left);
                self.check_module(stringify!($name), right);
                T::check_pair(stringify!($name), left.to_dynamic(), right.to_dynamic());
                // SAFETY: the checks above are what LLVM requires.
                let result = unsafe { raw::$build(self.raw, left.raw, right.raw, EMPTY.as_ptr()) };
                set_name(result, name);
                // SAFETY: an integer operation on two `T`s gives a `T`, live for `'module`.
                unsafe { Value::from_raw(result) }
            }
        )*
    };
}

/// Declares build calls that take two float operands.
macro_rules! float_binary {
    ($($(#[$meta:meta])* $name:ident = $build:ident;)*) => {
        $(
            $(#[$meta])*
            ///
            /// # Panics
            /// If an operand belongs to another module, the types differ, or the block is already terminated (D3).
            #[track_caller]
            pub fn $name<T: FloatOperand>(
                &self,
                left: Value<'context, 'module, T>,
                right: Value<'context, 'module, T>,
                name: &str,
            ) -> Value<'context, 'module, T> {
                self.check_open(stringify!($name));
                self.check_module(stringify!($name), left);
                self.check_module(stringify!($name), right);
                T::check_pair(stringify!($name), left.to_dynamic(), right.to_dynamic());
                // SAFETY: the checks above are what LLVM requires.
                let result = unsafe { raw::$build(self.raw, left.raw, right.raw, EMPTY.as_ptr()) };
                set_name(result, name);
                // SAFETY: a float operation on two `T`s gives a `T`, live for `'module`.
                unsafe { Value::from_raw(result) }
            }
        )*
    };
}

/// Declares build calls that take one integer operand.
macro_rules! integer_unary {
    ($($(#[$meta:meta])* $name:ident = $build:ident;)*) => {
        $(
            $(#[$meta])*
            ///
            /// # Panics
            /// If the operand belongs to another module or isn't an integer, or the block is already terminated (D3).
            #[track_caller]
            pub fn $name<T: IntegerOperand>(
                &self,
                value: Value<'context, 'module, T>,
                name: &str,
            ) -> Value<'context, 'module, T> {
                self.check_open(stringify!($name));
                self.check_module(stringify!($name), value);
                T::check_one(stringify!($name), value.to_dynamic());
                // SAFETY: the checks above are what LLVM requires.
                let result = unsafe { raw::$build(self.raw, value.raw, EMPTY.as_ptr()) };
                set_name(result, name);
                // SAFETY: an integer operation on a `T` gives a `T`, live for `'module`.
                unsafe { Value::from_raw(result) }
            }
        )*
    };
}

/// Declares casts between two kinds whose LLVM rules the shim's `castIsValid` can check when the line runs.
macro_rules! checked_cast {
    ($($(#[$meta:meta])* $name:ident = $build:ident: $opcode:ident;)*) => {
        $(
            $(#[$meta])*
            ///
            /// # Panics
            /// If LLVM's rules don't allow this cast, the value belongs to another module, or the block is already
            /// terminated (D3).
            #[track_caller]
            pub fn $name<S: ValueMarker, D: TypeMarker>(
                &self,
                value: Value<'context, 'module, S>,
                target: Type<'context, D>,
                name: &str,
            ) -> Value<'context, 'module, D> {
                self.check_open(stringify!($name));
                self.check_module(stringify!($name), value);
                Self::check_cast(stringify!($name), raw::LLVMOpcode::$opcode, value, target);
                // SAFETY: the checks above are what LLVM requires.
                let result = unsafe { raw::$build(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
                set_name(result, name);
                // SAFETY: the cast gives a value of the target type, live for `'module`.
                unsafe { Value::from_raw(result) }
            }
        )*
    };
}

impl<'context, 'module> Builder<'context, 'module> {
    /// A builder placed at the end of `block`.
    #[must_use]
    pub fn at_end_of(block: BasicBlock<'context, 'module>) -> Self {
        // SAFETY: the block is live for `'module`, is in a function, and that function is in a module.
        unsafe {
            let function = raw::LLVMGetBasicBlockParent(block.raw);
            let module = raw::LLVMGetGlobalParent(function);
            let builder = raw::LLVMCreateBuilderInContext(raw::LLVMGetModuleContext(module));
            raw::LLVMPositionBuilderAtEnd(builder, block.raw);
            Builder { raw: builder, module, function: Cell::new(function), marker: PhantomData }
        }
    }

    /// A builder placed just before `instruction`.
    #[must_use]
    pub fn before<T: InstructionMarker>(instruction: Value<'context, 'module, T>) -> Self {
        let builder = Self::at_end_of(instruction.parent_basic_block());
        // SAFETY: both are live, and the instruction is in the block the builder is in.
        unsafe { raw::LLVMPositionBuilderBefore(builder.raw, instruction.raw) };
        builder
    }

    /// A builder placed in `block` at its first place an ordinary instruction can go: after its phis and landing pad.
    #[must_use]
    pub fn at_first_insertion_point(block: BasicBlock<'context, 'module>) -> Self {
        let builder = Self::at_end_of(block);
        // SAFETY: both are live, and the builder is in this block.
        unsafe { shim::lyftar_builder_position_at_first_insertion_point(builder.raw, block.raw) };
        builder
    }

    /// The raw builder.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15), a builder with no position among them (D3).
    #[must_use]
    pub unsafe fn as_raw(&self) -> raw::LLVMBuilderRef {
        self.raw
    }

    /// Wraps a raw builder, taking ownership of it.
    ///
    /// # Safety
    /// `raw` must be a live builder placed in a block of a module that stays live for `'module`, and nothing else may
    /// own it.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMBuilderRef) -> Self {
        // SAFETY: the caller promises it is placed, so these are all there.
        unsafe {
            let function = raw::LLVMGetBasicBlockParent(raw::LLVMGetInsertBlock(raw));
            let module = raw::LLVMGetGlobalParent(function);
            Builder { raw, module, function: Cell::new(function), marker: PhantomData }
        }
    }

    /// Moves this builder to the end of `block`.
    ///
    /// # Panics
    /// If the block belongs to another module (D3).
    #[track_caller]
    pub fn position_at_end(&self, block: BasicBlock<'context, 'module>) {
        self.check_block_module("position_at_end", block);
        // SAFETY: both live, in one module.
        unsafe {
            raw::LLVMPositionBuilderAtEnd(self.raw, block.raw);
            self.function.set(raw::LLVMGetBasicBlockParent(block.raw));
        }
    }

    /// Moves this builder to just before `instruction`, and before its debug records.
    ///
    /// # Panics
    /// If the instruction belongs to another module (D3).
    #[track_caller]
    pub fn position_before<T: InstructionMarker>(&self, instruction: Value<'context, 'module, T>) {
        self.check_module("position_before", instruction);
        // SAFETY: both live, in one module.
        unsafe {
            raw::LLVMPositionBuilderBefore(self.raw, instruction.raw);
            self.function.set(raw::LLVMGetBasicBlockParent(raw::LLVMGetInstructionParent(instruction.raw)));
        }
    }

    /// Moves this builder to just before `instruction`, after its debug records.
    ///
    /// # Panics
    /// If the instruction belongs to another module (D3).
    #[track_caller]
    pub fn position_before_instruction_and_debug_records<T: InstructionMarker>(
        &self,
        instruction: Value<'context, 'module, T>,
    ) {
        self.check_module("position_before_instruction_and_debug_records", instruction);
        // SAFETY: both live, in one module.
        unsafe {
            raw::LLVMPositionBuilderBeforeInstrAndDbgRecords(self.raw, instruction.raw);
            self.function.set(raw::LLVMGetBasicBlockParent(raw::LLVMGetInstructionParent(instruction.raw)));
        }
    }

    /// Moves this builder into `block`, before `instruction` if given, and otherwise to the block's end.
    ///
    /// # Panics
    /// If they belong to another module, or the instruction isn't in the block (D3).
    #[track_caller]
    pub fn position<T: InstructionMarker>(
        &self,
        block: BasicBlock<'context, 'module>,
        instruction: Option<Value<'context, 'module, T>>,
    ) {
        self.check_block_module("position", block);
        let raw_instruction = match instruction {
            Some(found) => {
                if found.parent_basic_block() != block {
                    bad_call(format_args!("position: {} isn't in %{}", found.describe(), block.name()));
                }
                found.raw
            }
            None => std::ptr::null_mut(),
        };
        // SAFETY: just checked; a null instruction means the block's end.
        unsafe {
            raw::LLVMPositionBuilder(self.raw, block.raw, raw_instruction);
            self.function.set(raw::LLVMGetBasicBlockParent(block.raw));
        }
    }

    /// As [`Builder::position`], but placing the builder before any debug records in front of the instruction.
    ///
    /// # Panics
    /// If they belong to another module, or the instruction isn't in the block (D3).
    #[track_caller]
    pub fn position_before_debug_records<T: InstructionMarker>(
        &self,
        block: BasicBlock<'context, 'module>,
        instruction: Option<Value<'context, 'module, T>>,
    ) {
        self.check_block_module("position_before_debug_records", block);
        let raw_instruction = match instruction {
            Some(found) => {
                if found.parent_basic_block() != block {
                    bad_call(format_args!(
                        "position_before_debug_records: {} isn't in %{}",
                        found.describe(),
                        block.name()
                    ));
                }
                found.raw
            }
            None => std::ptr::null_mut(),
        };
        // SAFETY: just checked; a null instruction means the block's end.
        unsafe {
            raw::LLVMPositionBuilderBeforeDbgRecords(self.raw, block.raw, raw_instruction);
            self.function.set(raw::LLVMGetBasicBlockParent(block.raw));
        }
    }

    /// The block this builder is placed in.
    #[must_use]
    pub fn insertion_basic_block(&self) -> BasicBlock<'context, 'module> {
        // SAFETY: a `Builder` is always placed, in a block that is live for `'module`.
        unsafe { BasicBlock::from_raw(raw::LLVMGetInsertBlock(self.raw)) }
    }

    /// The debug location given to what this builder builds.
    #[must_use]
    pub fn current_debug_location(&self) -> Option<Metadata<'context, 'module>> {
        // SAFETY: the builder is live.
        let location = unsafe { raw::LLVMGetCurrentDebugLocation2(self.raw) };
        // SAFETY: LLVM gave back live metadata.
        (!location.is_null()).then(|| unsafe { Metadata::from_raw(location) })
    }

    /// Sets the debug location given to what this builder builds, or clears it.
    pub fn set_current_debug_location(&self, location: Option<Metadata<'context, 'module>>) {
        let raw_location = location.map_or(std::ptr::null_mut(), |metadata| metadata.raw);
        // SAFETY: the builder is live, and a null location clears it.
        unsafe { raw::LLVMSetCurrentDebugLocation2(self.raw, raw_location) };
    }

    /// Gives an instruction this builder's current debug location.
    pub fn set_instruction_debug_location<T: InstructionMarker>(&self, instruction: Value<'context, 'module, T>) {
        // SAFETY: both are live.
        unsafe { raw::LLVMSetInstDebugLocation(self.raw, instruction.raw) };
    }

    /// Gives an instruction all the metadata this builder gives what it builds.
    pub fn add_metadata_to_instruction<T: InstructionMarker>(&self, instruction: Value<'context, 'module, T>) {
        // SAFETY: both are live.
        unsafe { raw::LLVMAddMetadataToInst(self.raw, instruction.raw) };
    }

    /// The floating-point math tag given to what this builder builds.
    #[must_use]
    pub fn default_float_math_tag(&self) -> Option<Metadata<'context, 'module>> {
        // SAFETY: the builder is live.
        let tag = unsafe { raw::LLVMBuilderGetDefaultFPMathTag(self.raw) };
        // SAFETY: LLVM gave back live metadata.
        (!tag.is_null()).then(|| unsafe { Metadata::from_raw(tag) })
    }

    /// Sets the floating-point math tag given to what this builder builds, or clears it.
    pub fn set_default_float_math_tag(&self, tag: Option<Metadata<'context, 'module>>) {
        let raw_tag = tag.map_or(std::ptr::null_mut(), |metadata| metadata.raw);
        // SAFETY: the builder is live, and a null tag clears it.
        unsafe { raw::LLVMBuilderSetDefaultFPMathTag(self.raw, raw_tag) };
    }

    // trace: D3
    #[inline]
    #[track_caller]
    fn check_module<T>(&self, operation: &str, value: Value<'context, 'module, T>) {
        // SAFETY: both are live.
        if !unsafe { shim::lyftar_value_belongs_to_module(value.raw, self.module) } {
            bad_call(format_args!("{operation}: {} belongs to another module than this builder's", value.describe()));
        }
    }

    #[inline]
    #[track_caller]
    fn check_values_module<T>(&self, operation: &str, values: &[Value<'context, 'module, T>]) {
        for value in values {
            self.check_module(operation, *value);
        }
    }

    #[inline]
    #[track_caller]
    fn check_block_module(&self, operation: &str, block: BasicBlock<'context, 'module>) {
        // SAFETY: the block is live, in a function in a module.
        let module = unsafe { raw::LLVMGetGlobalParent(raw::LLVMGetBasicBlockParent(block.raw)) };
        if !checks::same_identity(module as usize, self.module as usize) {
            bad_call(format_args!(
                "{operation}: block %{} belongs to another module than this builder's",
                block.name()
            ));
        }
    }

    #[inline]
    #[track_caller]
    fn check_destination(&self, operation: &str, block: BasicBlock<'context, 'module>) {
        // SAFETY: the block is live and in a function.
        let function = unsafe { raw::LLVMGetBasicBlockParent(block.raw) };
        if !checks::same_identity(function as usize, self.function.get() as usize) {
            self.check_block_module(operation, block);
            bad_call(format_args!("{operation}: block %{} is in another function", block.name()));
        }
    }

    // trace: D3
    #[inline]
    #[track_caller]
    fn check_open(&self, operation: &str) {
        // SAFETY: the builder is live and placed.
        if !unsafe { shim::lyftar_builder_is_open(self.raw) } {
            bad_call(format_args!(
                "{operation}: the block already ends in a terminator, or the builder is before a phi"
            ));
        }
    }

    /// A terminator goes last, so it also needs the builder at the block's end.
    #[inline]
    #[track_caller]
    fn check_terminator_position(&self, operation: &str) {
        // SAFETY: the builder is live and placed.
        if !unsafe { shim::lyftar_builder_is_at_open_end(self.raw) } {
            bad_call(format_args!("{operation}: a terminator goes at the end of a block that doesn't have one yet"));
        }
    }

    #[track_caller]
    fn check_cast<S, D>(
        operation: &str,
        opcode: raw::LLVMOpcode,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
    ) {
        // SAFETY: both live; the shim only reads them.
        if !unsafe { shim::lyftar_cast_is_valid(opcode, value.raw, target.raw) } {
            bad_call(format_args!("{operation}: can't cast {} to {}", value.describe(), target.describe()));
        }
    }

    integer_binary! {
        /// Adds two integers, wrapping on overflow.
        integer_add = LLVMBuildAdd;
        /// Adds two integers, promising no signed wrap.
        integer_add_no_signed_wrap = LLVMBuildNSWAdd;
        /// Adds two integers, promising no unsigned wrap.
        integer_add_no_unsigned_wrap = LLVMBuildNUWAdd;
        /// Subtracts one integer from another, wrapping on overflow.
        integer_subtract = LLVMBuildSub;
        /// Subtracts one integer from another, promising no signed wrap.
        integer_subtract_no_signed_wrap = LLVMBuildNSWSub;
        /// Subtracts one integer from another, promising no unsigned wrap.
        integer_subtract_no_unsigned_wrap = LLVMBuildNUWSub;
        /// Multiplies two integers, wrapping on overflow.
        integer_multiply = LLVMBuildMul;
        /// Multiplies two integers, promising no signed wrap.
        integer_multiply_no_signed_wrap = LLVMBuildNSWMul;
        /// Multiplies two integers, promising no unsigned wrap.
        integer_multiply_no_unsigned_wrap = LLVMBuildNUWMul;
        /// Divides two unsigned integers.
        unsigned_divide = LLVMBuildUDiv;
        /// Divides two unsigned integers, promising no remainder.
        exact_unsigned_divide = LLVMBuildExactUDiv;
        /// Divides two signed integers.
        signed_divide = LLVMBuildSDiv;
        /// Divides two signed integers, promising no remainder.
        exact_signed_divide = LLVMBuildExactSDiv;
        /// The remainder of dividing two unsigned integers.
        unsigned_remainder = LLVMBuildURem;
        /// The remainder of dividing two signed integers.
        signed_remainder = LLVMBuildSRem;
        /// Shifts an integer left.
        shift_left = LLVMBuildShl;
        /// Shifts an integer right, filling with zeros.
        logical_shift_right = LLVMBuildLShr;
        /// Shifts an integer right, filling with the sign bit.
        arithmetic_shift_right = LLVMBuildAShr;
        /// The bits set in both.
        and = LLVMBuildAnd;
        /// The bits set in either.
        or = LLVMBuildOr;
        /// The bits set in exactly one.
        exclusive_or = LLVMBuildXor;
    }

    float_binary! {
        /// Adds two floats.
        float_add = LLVMBuildFAdd;
        /// Subtracts one float from another.
        float_subtract = LLVMBuildFSub;
        /// Multiplies two floats.
        float_multiply = LLVMBuildFMul;
        /// Divides two floats.
        float_divide = LLVMBuildFDiv;
        /// The remainder of dividing two floats.
        float_remainder = LLVMBuildFRem;
    }

    integer_unary! {
        /// `0 - value`.
        integer_negate = LLVMBuildNeg;
        /// `0 - value`, promising no signed wrap.
        integer_negate_no_signed_wrap = LLVMBuildNSWNeg;
        /// `0 - value`, promising no unsigned wrap.
        integer_negate_no_unsigned_wrap = LLVMBuildNUWNeg;
        /// Every bit flipped.
        not = LLVMBuildNot;
    }

    /// Negates a float.
    ///
    /// # Panics
    /// If the operand belongs to another module or isn't a float, or the block is already terminated (D3).
    #[track_caller]
    pub fn float_negate<T: FloatOperand>(
        &self,
        value: Value<'context, 'module, T>,
        name: &str,
    ) -> Value<'context, 'module, T> {
        self.check_open("float_negate");
        self.check_module("float_negate", value);
        T::check_one("float_negate", value.to_dynamic());
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildFNeg(self.raw, value.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: negating a `T` gives a `T`, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// A two-operand instruction chosen when the line runs.
    ///
    /// # Panics
    /// If `opcode` isn't a two-operand operation that takes these operands (D3).
    #[track_caller]
    pub fn binary_operation(
        &self,
        opcode: Opcode,
        left: Value<'context, 'module>,
        right: Value<'context, 'module>,
        name: &str,
    ) -> Value<'context, 'module> {
        self.check_open("binary_operation");
        self.check_module("binary_operation", left);
        self.check_module("binary_operation", right);
        // SAFETY: only reads them.
        if !unsafe { shim::lyftar_binary_operation_is_valid(opcode.to_raw(), left.raw, right.raw) } {
            bad_call(format_args!(
                "binary_operation: {opcode:?} doesn't take {} and {}",
                left.describe(),
                right.describe()
            ));
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildBinOp(self.raw, opcode.to_raw(), left.raw, right.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: LLVM gave back a value live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    // ---- Comparisons ----

    /// Compares two integers.
    ///
    /// # Panics
    /// If an operand belongs to another module, the types differ, or the block is already terminated (D3).
    #[track_caller]
    pub fn integer_compare<T: IntegerOperand>(
        &self,
        predicate: IntegerPredicate,
        left: Value<'context, 'module, T>,
        right: Value<'context, 'module, T>,
        name: &str,
    ) -> Value<'context, 'module, T::Comparison> {
        self.check_open("integer_compare");
        self.check_module("integer_compare", left);
        self.check_module("integer_compare", right);
        T::check_pair("integer_compare", left.to_dynamic(), right.to_dynamic());
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildICmp(self.raw, predicate.to_raw(), left.raw, right.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: comparing gives what `T::Comparison` says, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Compares two pointers.
    ///
    /// # Panics
    /// If an operand belongs to another module, the types differ, or the block is already terminated (D3).
    #[track_caller]
    pub fn pointer_compare<T: PointerOperand>(
        &self,
        predicate: IntegerPredicate,
        left: Value<'context, 'module, T>,
        right: Value<'context, 'module, T>,
        name: &str,
    ) -> Value<'context, 'module, Integer<Bits<1>>> {
        self.check_open("pointer_compare");
        self.check_module("pointer_compare", left);
        self.check_module("pointer_compare", right);
        T::check_pointer("pointer_compare", left.to_dynamic());
        check_same_type("pointer_compare", left.raw, right.raw);
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildICmp(self.raw, predicate.to_raw(), left.raw, right.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: comparing two single pointers gives an `i1`, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Compares two floats.
    ///
    /// # Panics
    /// If an operand belongs to another module, the types differ, or the block is already terminated (D3).
    #[track_caller]
    pub fn float_compare<T: FloatOperand>(
        &self,
        predicate: FloatPredicate,
        left: Value<'context, 'module, T>,
        right: Value<'context, 'module, T>,
        name: &str,
    ) -> Value<'context, 'module, T::Comparison> {
        self.check_open("float_compare");
        self.check_module("float_compare", left);
        self.check_module("float_compare", right);
        T::check_pair("float_compare", left.to_dynamic(), right.to_dynamic());
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildFCmp(self.raw, predicate.to_raw(), left.raw, right.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: comparing gives what `T::Comparison` says, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Whether a value is zero.
    ///
    /// # Panics
    /// If it belongs to another module, it has no zero value, or the block is already terminated (D3).
    #[track_caller]
    pub fn is_null<T>(
        &self,
        value: Value<'context, 'module, T>,
        name: &str,
    ) -> Value<'context, 'module, Integer<Bits<1>>> {
        self.check_open("is_null");
        self.check_module("is_null", value);
        check_integer_or_pointer("is_null", value.raw);
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildIsNull(self.raw, value.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: a single value's comparison with zero gives an `i1`, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Whether a value isn't zero.
    ///
    /// # Panics
    /// If it belongs to another module, it has no zero value, or the block is already terminated (D3).
    #[track_caller]
    pub fn is_not_null<T>(
        &self,
        value: Value<'context, 'module, T>,
        name: &str,
    ) -> Value<'context, 'module, Integer<Bits<1>>> {
        self.check_open("is_not_null");
        self.check_module("is_not_null", value);
        check_integer_or_pointer("is_not_null", value.raw);
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildIsNotNull(self.raw, value.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: a single value's comparison with zero gives an `i1`, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// How many elements of `element_type` lie between two pointers.
    ///
    /// # Panics
    /// If the pointers belong to another module, the element has no size, or the block is already terminated (D3).
    #[track_caller]
    pub fn pointer_difference<T: PointerOperand>(
        &self,
        element_type: Type<'context>,
        left: Value<'context, 'module, T>,
        right: Value<'context, 'module, T>,
        name: &str,
    ) -> Value<'context, 'module, Integer> {
        self.check_open("pointer_difference");
        self.check_module("pointer_difference", left);
        self.check_module("pointer_difference", right);
        T::check_pointer("pointer_difference", left.to_dynamic());
        check_same_type("pointer_difference", left.raw, right.raw);
        if !element_type.is_sized() {
            bad_call(format_args!("pointer_difference: {} has no size", element_type.describe()));
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildPtrDiff2(self.raw, element_type.raw, left.raw, right.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: a pointer difference is an integer of pointer size, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    // ---- Casts ----

    /// Cuts an integer to a narrower one.
    ///
    /// # Panics
    /// If the target isn't narrower, the value belongs to another module, or the block is already terminated. With both
    /// widths spelled out, the first of those fails at `cargo build` (D3, D4).
    #[track_caller]
    pub fn truncate<S: IntegerOperand, D: IntegerOperand>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        const {
            if let (Some(source), Some(destination)) = (S::WIDTH_IN_BITS, D::WIDTH_IN_BITS) {
                assert!(destination < source, "truncate: the target must be narrower than the value");
            }
        };
        self.check_open("truncate");
        self.check_module("truncate", value);
        if S::WIDTH_IN_BITS.is_none() || D::WIDTH_IN_BITS.is_none() {
            Self::check_cast("truncate", raw::LLVMOpcode::LLVMTrunc, value, target);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildTrunc(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Widens an integer, filling with zeros.
    ///
    /// # Panics
    /// If the target isn't wider, the value belongs to another module, or the block is already terminated. With both
    /// widths spelled out, the first of those fails at `cargo build` (D3, D4).
    #[track_caller]
    pub fn zero_extend<S: IntegerOperand, D: IntegerOperand>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        const {
            if let (Some(source), Some(destination)) = (S::WIDTH_IN_BITS, D::WIDTH_IN_BITS) {
                assert!(source < destination, "zero_extend: the target must be wider than the value");
            }
        };
        self.check_open("zero_extend");
        self.check_module("zero_extend", value);
        if S::WIDTH_IN_BITS.is_none() || D::WIDTH_IN_BITS.is_none() {
            Self::check_cast("zero_extend", raw::LLVMOpcode::LLVMZExt, value, target);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildZExt(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Widens an integer, filling with its sign bit.
    ///
    /// # Panics
    /// If the target isn't wider, the value belongs to another module, or the block is already terminated. With both
    /// widths spelled out, the first of those fails at `cargo build` (D3, D4).
    #[track_caller]
    pub fn sign_extend<S: IntegerOperand, D: IntegerOperand>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        const {
            if let (Some(source), Some(destination)) = (S::WIDTH_IN_BITS, D::WIDTH_IN_BITS) {
                assert!(source < destination, "sign_extend: the target must be wider than the value");
            }
        };
        self.check_open("sign_extend");
        self.check_module("sign_extend", value);
        if S::WIDTH_IN_BITS.is_none() || D::WIDTH_IN_BITS.is_none() {
            Self::check_cast("sign_extend", raw::LLVMOpcode::LLVMSExt, value, target);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildSExt(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Narrows a float to a smaller format.
    ///
    /// # Panics
    /// If the target isn't smaller, the value belongs to another module, or the block is already terminated. With both
    /// formats spelled out, the first of those fails at `cargo build` (D3, D4).
    #[track_caller]
    pub fn float_truncate<S: FloatOperand, D: FloatOperand>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        const {
            if let (Some(source), Some(destination)) = (S::SIZE_IN_BITS, D::SIZE_IN_BITS) {
                assert!(destination < source, "float_truncate: the target must be smaller than the value");
            }
        };
        self.check_open("float_truncate");
        self.check_module("float_truncate", value);
        if S::SIZE_IN_BITS.is_none() || D::SIZE_IN_BITS.is_none() {
            Self::check_cast("float_truncate", raw::LLVMOpcode::LLVMFPTrunc, value, target);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildFPTrunc(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Widens a float to a larger format.
    ///
    /// # Panics
    /// If the target isn't larger, the value belongs to another module, or the block is already terminated. With both
    /// formats spelled out, the first of those fails at `cargo build` (D3, D4).
    #[track_caller]
    pub fn float_extend<S: FloatOperand, D: FloatOperand>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        const {
            if let (Some(source), Some(destination)) = (S::SIZE_IN_BITS, D::SIZE_IN_BITS) {
                assert!(source < destination, "float_extend: the target must be larger than the value");
            }
        };
        self.check_open("float_extend");
        self.check_module("float_extend", value);
        if S::SIZE_IN_BITS.is_none() || D::SIZE_IN_BITS.is_none() {
            Self::check_cast("float_extend", raw::LLVMOpcode::LLVMFPExt, value, target);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildFPExt(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Casts a float to an unsigned integer.
    ///
    /// # Panics
    /// If the value belongs to another module, or the block is already terminated, or a dynamic operand isn't one (D3).
    #[track_caller]
    pub fn float_to_unsigned_integer<S: FloatOperand, D: IntegerOperand>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_open("float_to_unsigned_integer");
        self.check_module("float_to_unsigned_integer", value);
        if S::SIZE_IN_BITS.is_none() || D::WIDTH_IN_BITS.is_none() {
            Self::check_cast("float_to_unsigned_integer", raw::LLVMOpcode::LLVMFPToUI, value, target);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildFPToUI(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Casts a float to a signed integer.
    ///
    /// # Panics
    /// If the value belongs to another module, or the block is already terminated, or a dynamic operand isn't one (D3).
    #[track_caller]
    pub fn float_to_signed_integer<S: FloatOperand, D: IntegerOperand>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_open("float_to_signed_integer");
        self.check_module("float_to_signed_integer", value);
        if S::SIZE_IN_BITS.is_none() || D::WIDTH_IN_BITS.is_none() {
            Self::check_cast("float_to_signed_integer", raw::LLVMOpcode::LLVMFPToSI, value, target);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildFPToSI(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Casts an unsigned integer to a float.
    ///
    /// # Panics
    /// If the value belongs to another module, or the block is already terminated, or a dynamic operand isn't one (D3).
    #[track_caller]
    pub fn unsigned_integer_to_float<S: IntegerOperand, D: FloatOperand>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_open("unsigned_integer_to_float");
        self.check_module("unsigned_integer_to_float", value);
        if S::WIDTH_IN_BITS.is_none() || D::SIZE_IN_BITS.is_none() {
            Self::check_cast("unsigned_integer_to_float", raw::LLVMOpcode::LLVMUIToFP, value, target);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildUIToFP(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Casts a signed integer to a float.
    ///
    /// # Panics
    /// If the value belongs to another module, or the block is already terminated, or a dynamic operand isn't one (D3).
    #[track_caller]
    pub fn signed_integer_to_float<S: IntegerOperand, D: FloatOperand>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_open("signed_integer_to_float");
        self.check_module("signed_integer_to_float", value);
        if S::WIDTH_IN_BITS.is_none() || D::SIZE_IN_BITS.is_none() {
            Self::check_cast("signed_integer_to_float", raw::LLVMOpcode::LLVMSIToFP, value, target);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildSIToFP(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Casts a pointer to an integer.
    ///
    /// # Panics
    /// If the value belongs to another module, or the block is already terminated, or a dynamic operand isn't one (D3).
    #[track_caller]
    pub fn pointer_to_integer<S: PointerOperand, D: IntegerOperand>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_open("pointer_to_integer");
        self.check_module("pointer_to_integer", value);
        if S::IS_DYNAMIC || D::WIDTH_IN_BITS.is_none() {
            Self::check_cast("pointer_to_integer", raw::LLVMOpcode::LLVMPtrToInt, value, target);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildPtrToInt(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Casts an integer to a pointer.
    ///
    /// # Panics
    /// If the value belongs to another module, or the block is already terminated, or a dynamic operand isn't one (D3).
    #[track_caller]
    pub fn integer_to_pointer<S: IntegerOperand, D: TypeMarker>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_open("integer_to_pointer");
        self.check_module("integer_to_pointer", value);
        Self::check_cast("integer_to_pointer", raw::LLVMOpcode::LLVMIntToPtr, value, target);
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildIntToPtr(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    checked_cast! {
        /// Reinterprets a value's bits as another type of the same size.
        bit_cast = LLVMBuildBitCast: LLVMBitCast;
        /// Casts a pointer to another address space.
        address_space_cast = LLVMBuildAddrSpaceCast: LLVMAddrSpaceCast;
    }

    /// Zero-extends a value if the target is wider, and bit-casts it if they are the same size.
    ///
    /// # Panics
    /// If LLVM's rules allow neither, the value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn zero_extend_or_bit_cast<S: ValueMarker, D: TypeMarker>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_either_cast("zero_extend_or_bit_cast", raw::LLVMOpcode::LLVMZExt, value, target);
        // SAFETY: the check above is what LLVM requires.
        let result = unsafe { raw::LLVMBuildZExtOrBitCast(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Sign-extends a value if the target is wider, and bit-casts it if they are the same size.
    ///
    /// # Panics
    /// If LLVM's rules allow neither, the value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn sign_extend_or_bit_cast<S: ValueMarker, D: TypeMarker>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_either_cast("sign_extend_or_bit_cast", raw::LLVMOpcode::LLVMSExt, value, target);
        // SAFETY: the check above is what LLVM requires.
        let result = unsafe { raw::LLVMBuildSExtOrBitCast(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Truncates a value if the target is narrower, and bit-casts it if they are the same size.
    ///
    /// # Panics
    /// If LLVM's rules allow neither, the value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn truncate_or_bit_cast<S: ValueMarker, D: TypeMarker>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_either_cast("truncate_or_bit_cast", raw::LLVMOpcode::LLVMTrunc, value, target);
        // SAFETY: the check above is what LLVM requires.
        let result = unsafe { raw::LLVMBuildTruncOrBitCast(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    #[track_caller]
    fn check_either_cast<S, D>(
        &self,
        operation: &str,
        opcode: raw::LLVMOpcode,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
    ) {
        self.check_open(operation);
        self.check_module(operation, value);
        // SAFETY: both live; the shim only reads them.
        let valid = unsafe {
            shim::lyftar_cast_is_valid(opcode, value.raw, target.raw)
                || shim::lyftar_cast_is_valid(raw::LLVMOpcode::LLVMBitCast, value.raw, target.raw)
        };
        if !valid {
            bad_call(format_args!("{operation}: can't cast {} to {}", value.describe(), target.describe()));
        }
    }

    /// A cast chosen when the line runs.
    ///
    /// # Panics
    /// If LLVM's rules don't allow this cast, the value belongs to another module, or the block is already terminated
    /// (D3).
    #[track_caller]
    pub fn cast<S, D: TypeMarker>(
        &self,
        opcode: Opcode,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_open("cast");
        self.check_module("cast", value);
        Self::check_cast("cast", opcode.to_raw(), value, target);
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildCast(self.raw, opcode.to_raw(), value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Casts a pointer by whichever cast fits: to an integer, or to another pointer type.
    ///
    /// # Panics
    /// If the value isn't a pointer, or the target is neither, or the value belongs to another module, or the block is
    /// already terminated (D3).
    #[track_caller]
    pub fn pointer_cast<S: PointerOperand, D: TypeMarker>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_open("pointer_cast");
        self.check_module("pointer_cast", value);
        S::check_pointer("pointer_cast", value.to_dynamic());
        // SAFETY: both live; the shim only reads them.
        let valid = unsafe {
            shim::lyftar_cast_is_valid(raw::LLVMOpcode::LLVMPtrToInt, value.raw, target.raw)
                || shim::lyftar_cast_is_valid(raw::LLVMOpcode::LLVMBitCast, value.raw, target.raw)
                || shim::lyftar_cast_is_valid(raw::LLVMOpcode::LLVMAddrSpaceCast, value.raw, target.raw)
        };
        if !valid {
            bad_call(format_args!("pointer_cast: can't cast {} to {}", value.describe(), target.describe()));
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildPointerCast(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Casts an integer to another width: truncating, or extending by its sign if `is_signed` says so.
    ///
    /// # Panics
    /// If they aren't integers, the value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn integer_cast<S: IntegerOperand, D: IntegerOperand>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        is_signed: bool,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_open("integer_cast");
        self.check_module("integer_cast", value);
        S::check_one("integer_cast", value.to_dynamic());
        // SAFETY: both live.
        if !is_integer_like(target.raw) || !same_shape(unsafe { raw::LLVMTypeOf(value.raw) }, target.raw) {
            bad_call(format_args!("integer_cast: can't cast {} to {}", value.describe(), target.describe()));
        }
        // SAFETY: the checks above are what LLVM requires.
        let result =
            unsafe { raw::LLVMBuildIntCast2(self.raw, value.raw, target.raw, i32::from(is_signed), EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Casts a float to another format: truncating or extending.
    ///
    /// # Panics
    /// If they aren't floats, the value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn float_cast<S: FloatOperand, D: FloatOperand>(
        &self,
        value: Value<'context, 'module, S>,
        target: Type<'context, D>,
        name: &str,
    ) -> Value<'context, 'module, D> {
        self.check_open("float_cast");
        self.check_module("float_cast", value);
        S::check_one("float_cast", value.to_dynamic());
        // SAFETY: both live.
        if !is_float_like(target.raw) || !same_shape(unsafe { raw::LLVMTypeOf(value.raw) }, target.raw) {
            bad_call(format_args!("float_cast: can't cast {} to {}", value.describe(), target.describe()));
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildFPCast(self.raw, value.raw, target.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the cast gives a value of the target type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    // ---- Memory ----

    /// Allocates a `T` on the stack, in this function's frame.
    ///
    /// # Panics
    /// If the type has no size, or the block is already terminated (D3).
    #[track_caller]
    pub fn stack_allocate<T>(
        &self,
        allocated_type: Type<'context, T>,
        name: &str,
    ) -> Value<'context, 'module, StackAllocationInstruction> {
        self.check_open("stack_allocate");
        check_sized("stack_allocate", allocated_type);
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildAlloca(self.raw, allocated_type.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: LLVM just built it, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Allocates `count` of `T` on the stack.
    ///
    /// # Panics
    /// If the type has no size, `count` isn't an integer, or the block is already terminated (D3).
    #[track_caller]
    pub fn stack_allocate_array<T, C: IntegerOperand>(
        &self,
        allocated_type: Type<'context, T>,
        count: Value<'context, 'module, C>,
        name: &str,
    ) -> Value<'context, 'module, StackAllocationInstruction> {
        self.check_open("stack_allocate_array");
        check_sized("stack_allocate_array", allocated_type);
        self.check_module("stack_allocate_array", count);
        check_single_integer("stack_allocate_array", count.raw);
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildArrayAlloca(self.raw, allocated_type.raw, count.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: LLVM just built it, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Allocates a `T` with `malloc`, declaring it if needed.
    ///
    /// # Panics
    /// If the type has no size, or the block is already terminated (D3).
    #[track_caller]
    pub fn heap_allocate<T>(&self, allocated_type: Type<'context, T>, name: &str) -> Value<'context, 'module, Pointer> {
        self.check_open("heap_allocate");
        check_sized("heap_allocate", allocated_type);
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildMalloc(self.raw, allocated_type.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: `malloc` gives a pointer, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Allocates `count` of `T` with `malloc`, declaring it if needed.
    ///
    /// # Panics
    /// If the type has no size, `count` isn't an integer, or the block is already terminated (D3).
    #[track_caller]
    pub fn heap_allocate_array<T, C: IntegerOperand>(
        &self,
        allocated_type: Type<'context, T>,
        count: Value<'context, 'module, C>,
        name: &str,
    ) -> Value<'context, 'module, Pointer> {
        self.check_open("heap_allocate_array");
        check_sized("heap_allocate_array", allocated_type);
        self.check_module("heap_allocate_array", count);
        check_single_integer("heap_allocate_array", count.raw);
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildArrayMalloc(self.raw, allocated_type.raw, count.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: `malloc` gives a pointer, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Frees a pointer with `free`, declaring it if needed.
    ///
    /// # Panics
    /// If it isn't a pointer, it belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn heap_free<P: PointerOperand>(&self, pointer: Value<'context, 'module, P>) -> Value<'context, 'module> {
        self.check_open("heap_free");
        self.check_module("heap_free", pointer);
        P::check_pointer("heap_free", pointer.to_dynamic());
        // SAFETY: the checks above are what LLVM requires.
        unsafe { Value::from_raw(raw::LLVMBuildFree(self.raw, pointer.raw)) }
    }

    /// Fills memory with a byte.
    ///
    /// # Panics
    /// If the pointer or length is the wrong kind, the alignment isn't zero or a power of two, a value belongs to another
    /// module, or the block is already terminated (D3).
    #[track_caller]
    pub fn memory_set<P: PointerOperand, L: IntegerOperand>(
        &self,
        pointer: Value<'context, 'module, P>,
        byte: Value<'context, 'module, Integer<Bits<8>>>,
        length_in_bytes: Value<'context, 'module, L>,
        alignment_in_bytes: u32,
    ) -> Value<'context, 'module, CallInstruction> {
        self.check_open("memory_set");
        self.check_module("memory_set", pointer);
        self.check_module("memory_set", byte);
        self.check_module("memory_set", length_in_bytes);
        P::check_pointer("memory_set", pointer.to_dynamic());
        check_single_integer("memory_set", length_in_bytes.raw);
        check_optional_alignment("memory_set", alignment_in_bytes);
        // SAFETY: the checks above are what LLVM requires.
        unsafe {
            Value::from_raw(raw::LLVMBuildMemSet(
                self.raw,
                pointer.raw,
                byte.raw,
                length_in_bytes.raw,
                alignment_in_bytes,
            ))
        }
    }

    /// Copies memory that doesn't overlap.
    ///
    /// # Panics
    /// If an operand is the wrong kind, an alignment isn't zero or a power of two, a value belongs to another module, or
    /// the block is already terminated (D3).
    #[track_caller]
    pub fn memory_copy<D: PointerOperand, S: PointerOperand, L: IntegerOperand>(
        &self,
        destination: Value<'context, 'module, D>,
        destination_alignment_in_bytes: u32,
        source: Value<'context, 'module, S>,
        source_alignment_in_bytes: u32,
        length_in_bytes: Value<'context, 'module, L>,
    ) -> Value<'context, 'module, CallInstruction> {
        self.check_memory_transfer(
            "memory_copy",
            destination,
            destination_alignment_in_bytes,
            source,
            source_alignment_in_bytes,
            length_in_bytes,
        );
        // SAFETY: the checks above are what LLVM requires.
        unsafe {
            Value::from_raw(raw::LLVMBuildMemCpy(
                self.raw,
                destination.raw,
                destination_alignment_in_bytes,
                source.raw,
                source_alignment_in_bytes,
                length_in_bytes.raw,
            ))
        }
    }

    /// Copies memory that may overlap.
    ///
    /// # Panics
    /// If an operand is the wrong kind, an alignment isn't zero or a power of two, a value belongs to another module, or
    /// the block is already terminated (D3).
    #[track_caller]
    pub fn memory_move<D: PointerOperand, S: PointerOperand, L: IntegerOperand>(
        &self,
        destination: Value<'context, 'module, D>,
        destination_alignment_in_bytes: u32,
        source: Value<'context, 'module, S>,
        source_alignment_in_bytes: u32,
        length_in_bytes: Value<'context, 'module, L>,
    ) -> Value<'context, 'module, CallInstruction> {
        self.check_memory_transfer(
            "memory_move",
            destination,
            destination_alignment_in_bytes,
            source,
            source_alignment_in_bytes,
            length_in_bytes,
        );
        // SAFETY: the checks above are what LLVM requires.
        unsafe {
            Value::from_raw(raw::LLVMBuildMemMove(
                self.raw,
                destination.raw,
                destination_alignment_in_bytes,
                source.raw,
                source_alignment_in_bytes,
                length_in_bytes.raw,
            ))
        }
    }

    #[track_caller]
    fn check_memory_transfer<D: PointerOperand, S: PointerOperand, L: IntegerOperand>(
        &self,
        operation: &str,
        destination: Value<'context, 'module, D>,
        destination_alignment_in_bytes: u32,
        source: Value<'context, 'module, S>,
        source_alignment_in_bytes: u32,
        length_in_bytes: Value<'context, 'module, L>,
    ) {
        self.check_open(operation);
        self.check_module(operation, destination);
        self.check_module(operation, source);
        self.check_module(operation, length_in_bytes);
        D::check_pointer(operation, destination.to_dynamic());
        S::check_pointer(operation, source.to_dynamic());
        check_single_integer(operation, length_in_bytes.raw);
        check_optional_alignment(operation, destination_alignment_in_bytes);
        check_optional_alignment(operation, source_alignment_in_bytes);
    }

    /// Loads a `T` through a pointer.
    ///
    /// # Panics
    /// If the type can't be loaded, the pointer isn't one, it belongs to another module, or the block is already terminated
    /// (D3).
    #[track_caller]
    pub fn load<T: TypeMarker, P: PointerOperand>(
        &self,
        loaded_type: Type<'context, T>,
        pointer: Value<'context, 'module, P>,
        name: &str,
    ) -> Value<'context, 'module, T> {
        self.check_open("load");
        self.check_module("load", pointer);
        P::check_pointer("load", pointer.to_dynamic());
        if !T::EXACT {
            check_loadable("load", loaded_type);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildLoad2(self.raw, loaded_type.raw, pointer.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: a load of a `T` gives a `T`, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Stores a value through a pointer.
    ///
    /// # Panics
    /// If the value's type can't be stored, the pointer isn't one, a value belongs to another module, or the block is
    /// already terminated (D3).
    #[track_caller]
    pub fn store<T: ValueMarker, P: PointerOperand>(
        &self,
        value: Value<'context, 'module, T>,
        pointer: Value<'context, 'module, P>,
    ) -> Value<'context, 'module, StoreInstruction> {
        self.check_open("store");
        self.check_module("store", value);
        self.check_module("store", pointer);
        P::check_pointer("store", pointer.to_dynamic());
        if !<T::LlvmType as TypeMarker>::EXACT {
            // SAFETY: the value is live.
            check_loadable("store", unsafe { Type::<'context, Dynamic>::from_raw(raw::LLVMTypeOf(value.raw)) });
        }
        // SAFETY: the checks above are what LLVM requires.
        unsafe { Value::from_raw(raw::LLVMBuildStore(self.raw, value.raw, pointer.raw)) }
    }

    /// Computes an address into a `source_type` from a pointer and indices, with no load: `getelementptr`.
    ///
    /// # Panics
    /// If the indices don't fit the type, the pointer isn't one, a value belongs to another module, or the block is already
    /// terminated (D3).
    #[track_caller]
    pub fn get_element_pointer<P: PointerOperand, I: IntegerOperand>(
        &self,
        source_type: Type<'context>,
        pointer: Value<'context, 'module, P>,
        indices: &[Value<'context, 'module, I>],
        name: &str,
    ) -> Value<'context, 'module, P::GetElementPointerResult> {
        let count = self.check_get_element_pointer::<P, I>("get_element_pointer", source_type, pointer, indices);
        // SAFETY: the checks above are what LLVM requires; it only reads the indices.
        let result = unsafe {
            raw::LLVMBuildGEP2(
                self.raw,
                source_type.raw,
                pointer.raw,
                indices.as_ptr().cast_mut().cast(),
                count,
                EMPTY.as_ptr(),
            )
        };
        set_name(result, name);
        // SAFETY: LLVM gave back what `P::GetElementPointerResult` says, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// As [`Builder::get_element_pointer`], promising the address stays in bounds of the object: `inbounds`.
    ///
    /// # Panics
    /// If the indices don't fit the type, the pointer isn't one, a value belongs to another module, or the block is already
    /// terminated (D3).
    #[track_caller]
    pub fn in_bounds_get_element_pointer<P: PointerOperand, I: IntegerOperand>(
        &self,
        source_type: Type<'context>,
        pointer: Value<'context, 'module, P>,
        indices: &[Value<'context, 'module, I>],
        name: &str,
    ) -> Value<'context, 'module, P::GetElementPointerResult> {
        let count =
            self.check_get_element_pointer::<P, I>("in_bounds_get_element_pointer", source_type, pointer, indices);
        // SAFETY: the checks above are what LLVM requires; it only reads the indices.
        let result = unsafe {
            raw::LLVMBuildInBoundsGEP2(
                self.raw,
                source_type.raw,
                pointer.raw,
                indices.as_ptr().cast_mut().cast(),
                count,
                EMPTY.as_ptr(),
            )
        };
        set_name(result, name);
        // SAFETY: LLVM gave back what `P::GetElementPointerResult` says, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// As [`Builder::get_element_pointer`], with the given no-wrap flags.
    ///
    /// # Panics
    /// If the indices don't fit the type, the pointer isn't one, a value belongs to another module, or the block is already
    /// terminated (D3).
    #[track_caller]
    pub fn get_element_pointer_with_flags<P: PointerOperand, I: IntegerOperand>(
        &self,
        source_type: Type<'context>,
        pointer: Value<'context, 'module, P>,
        indices: &[Value<'context, 'module, I>],
        flags: GetElementPointerFlags,
        name: &str,
    ) -> Value<'context, 'module, P::GetElementPointerResult> {
        let count =
            self.check_get_element_pointer::<P, I>("get_element_pointer_with_flags", source_type, pointer, indices);
        // SAFETY: the checks above are what LLVM requires; it only reads the indices.
        let result = unsafe {
            raw::LLVMBuildGEPWithNoWrapFlags(
                self.raw,
                source_type.raw,
                pointer.raw,
                indices.as_ptr().cast_mut().cast(),
                count,
                EMPTY.as_ptr(),
                flags.0,
            )
        };
        set_name(result, name);
        // SAFETY: LLVM gave back what `P::GetElementPointerResult` says, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    #[track_caller]
    fn check_get_element_pointer<P: PointerOperand, I: IntegerOperand>(
        &self,
        operation: &str,
        source_type: Type<'context>,
        pointer: Value<'context, 'module, P>,
        indices: &[Value<'context, 'module, I>],
    ) -> u32 {
        self.check_open(operation);
        self.check_module(operation, pointer);
        P::check_pointer_or_vector(operation, pointer.to_dynamic());
        self.check_values_module(operation, indices);
        let count = length_as_u32(indices.len(), operation);
        // SAFETY: only reads them; `Value` has the layout of `LLVMValueRef`.
        let valid = unsafe {
            shim::lyftar_get_element_pointer_is_valid(source_type.raw, indices.as_ptr().cast(), count, P::IS_DYNAMIC)
        };
        if !valid {
            bad_call(format_args!("{operation}: those indices don't fit {}", source_type.describe()));
        }
        count
    }

    /// Computes the address of a struct's element at `index`.
    ///
    /// # Panics
    /// If the type isn't a struct or has no such element, the pointer isn't one, it belongs to another module, or the block
    /// is already terminated (D3).
    #[track_caller]
    pub fn struct_get_element_pointer<P: PointerOperand>(
        &self,
        struct_type: Type<'context>,
        pointer: Value<'context, 'module, P>,
        index: u32,
        name: &str,
    ) -> Value<'context, 'module, P::GetElementPointerResult> {
        self.check_open("struct_get_element_pointer");
        self.check_module("struct_get_element_pointer", pointer);
        P::check_pointer("struct_get_element_pointer", pointer.to_dynamic());
        let count = struct_type.struct_element_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!(
                "struct_get_element_pointer: index {index} is out of bounds for {}",
                struct_type.describe()
            ));
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildStructGEP2(self.raw, struct_type.raw, pointer.raw, index, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: LLVM gave back what `P::GetElementPointerResult` says, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Adds a private global holding `text` and a NUL, and gives its address.
    ///
    /// # Panics
    /// If `text` has a NUL byte inside it, or the block is already terminated (D3).
    #[track_caller]
    pub fn global_string(&self, text: &str, name: &str) -> Value<'context, 'module, GlobalVariable> {
        self.check_open("global_string");
        let text = c_string(text, "global_string");
        // SAFETY: the text is a valid C string for the call, and the builder is in a function in a module.
        let result = unsafe { raw::LLVMBuildGlobalString(self.raw, text.as_ptr(), EMPTY.as_ptr()) };
        set_global_name(result, name);
        // SAFETY: LLVM just added this global, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Adds a private global holding `text` and a NUL, and gives a pointer to its first byte.
    ///
    /// # Panics
    /// If `text` has a NUL byte inside it, or the block is already terminated (D3).
    #[track_caller]
    pub fn global_string_pointer(&self, text: &str, name: &str) -> Value<'context, 'module, Pointer> {
        self.check_open("global_string_pointer");
        let text = c_string(text, "global_string_pointer");
        // SAFETY: the text is a valid C string for the call, and the builder is in a function in a module.
        let result = unsafe { raw::LLVMBuildGlobalStringPtr(self.raw, text.as_ptr(), EMPTY.as_ptr()) };
        set_global_name(result, name);
        // SAFETY: a global string's address is a pointer, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    // ---- Other instructions ----

    /// A `phi` of type `T`, to give incoming values to afterwards.
    ///
    /// # Panics
    /// If the type can't be a phi's, or the builder is placed after something other than a phi (D3).
    #[track_caller]
    pub fn phi<T: TypeMarker>(&self, phi_type: Type<'context, T>, name: &str) -> Value<'context, 'module, PhiNode<T>> {
        self.check_open("phi");
        // SAFETY: the builder is live and placed.
        if !unsafe { shim::lyftar_builder_can_insert_phi(self.raw) } {
            bad_call(format_args!("phi: phis must come first in a block"));
        }
        if !T::EXACT {
            check_first_class("phi", phi_type);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildPhi(self.raw, phi_type.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: a phi of a `T` is a `PhiNode<T>`, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    #[track_caller]
    fn check_call<A>(
        &self,
        operation: &str,
        function_type: Type<'context>,
        arguments: &[Value<'context, 'module, A>],
    ) -> u32 {
        if function_type.raw_kind() != raw::LLVMTypeKind::LLVMFunctionTypeKind {
            bad_call(format_args!("{operation}: {} is not a function type", function_type.describe()));
        }
        let count = length_as_u32(arguments.len(), operation);
        // SAFETY: only reads them; `Value` has the layout of `LLVMValueRef`.
        let bad_argument =
            unsafe { shim::lyftar_call_first_bad_argument(function_type.raw, arguments.as_ptr().cast(), count) };
        if bad_argument == CALL_HAS_WRONG_COUNT {
            bad_call(format_args!("{operation}: {} can't take {count} arguments", function_type.describe()));
        }
        if bad_argument != CALL_IS_VALID {
            bad_call(format_args!(
                "{operation}: argument {bad_argument}, {}, doesn't have the type {} wants",
                arguments[bad_argument as usize].describe(),
                function_type.describe()
            ));
        }
        self.check_values_module(operation, arguments);
        count
    }

    /// Calls a function directly.
    ///
    /// # Panics
    /// If the arguments don't fit its type, a value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn call<A>(
        &self,
        function: Value<'context, 'module, Function>,
        arguments: &[Value<'context, 'module, A>],
        name: &str,
    ) -> Value<'context, 'module, CallInstruction> {
        self.check_open("call");
        self.check_module("call", function);
        // SAFETY: the marker guarantees a function.
        let function_type: Type<'context> = unsafe { Type::from_raw(raw::LLVMGlobalGetValueType(function.raw)) };
        let count = self.check_call("call", function_type, arguments);
        // SAFETY: the checks above are what LLVM requires; it only reads the arguments.
        let result = unsafe {
            raw::LLVMBuildCall2(
                self.raw,
                function_type.raw,
                function.raw,
                arguments.as_ptr().cast_mut().cast(),
                count,
                EMPTY.as_ptr(),
            )
        };
        set_name(result, name);
        // SAFETY: LLVM just built a call, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Calls through a pointer, or inline assembly.
    ///
    /// # Panics
    /// If `function_type` isn't one, the arguments don't fit it, a value belongs to another module, or the block is already
    /// terminated (D3).
    #[track_caller]
    pub fn call_indirect<P: PointerOperand, A>(
        &self,
        function_type: Type<'context>,
        callee: Value<'context, 'module, P>,
        arguments: &[Value<'context, 'module, A>],
        name: &str,
    ) -> Value<'context, 'module, CallInstruction> {
        self.check_open("call_indirect");
        self.check_module("call_indirect", callee);
        P::check_pointer("call_indirect", callee.to_dynamic());
        let count = self.check_call("call_indirect", function_type, arguments);
        // SAFETY: the checks above are what LLVM requires; it only reads the arguments.
        let result = unsafe {
            raw::LLVMBuildCall2(
                self.raw,
                function_type.raw,
                callee.raw,
                arguments.as_ptr().cast_mut().cast(),
                count,
                EMPTY.as_ptr(),
            )
        };
        set_name(result, name);
        // SAFETY: LLVM just built a call, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Calls through a pointer, with operand bundles.
    ///
    /// # Panics
    /// If `function_type` isn't one, the arguments don't fit it, a value belongs to another module, or the block is already
    /// terminated (D3).
    #[track_caller]
    pub fn call_with_operand_bundles<P: PointerOperand, A>(
        &self,
        function_type: Type<'context>,
        callee: Value<'context, 'module, P>,
        arguments: &[Value<'context, 'module, A>],
        bundles: &[OperandBundle<'context, 'module>],
        name: &str,
    ) -> Value<'context, 'module, CallInstruction> {
        self.check_open("call_with_operand_bundles");
        self.check_module("call_with_operand_bundles", callee);
        P::check_pointer("call_with_operand_bundles", callee.to_dynamic());
        let count = self.check_call("call_with_operand_bundles", function_type, arguments);
        self.check_bundles("call_with_operand_bundles", bundles);
        // SAFETY: the checks above are what LLVM requires; it only reads the arguments and bundles.
        let result = unsafe {
            raw::LLVMBuildCallWithOperandBundles(
                self.raw,
                function_type.raw,
                callee.raw,
                arguments.as_ptr().cast_mut().cast(),
                count,
                bundles.as_ptr().cast_mut().cast(),
                length_as_u32(bundles.len(), "call_with_operand_bundles"),
                EMPTY.as_ptr(),
            )
        };
        set_name(result, name);
        // SAFETY: LLVM just built a call, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    #[track_caller]
    fn check_bundles(&self, operation: &str, bundles: &[OperandBundle<'context, 'module>]) {
        for bundle in bundles {
            self.check_values_module(operation, &bundle.arguments());
        }
    }

    /// Picks one of two values by a condition.
    ///
    /// # Panics
    /// If LLVM's rules don't allow these operands, a value belongs to another module, or the block is already terminated
    /// (D3).
    #[track_caller]
    pub fn select<C: ValueMarker, T: ValueMarker>(
        &self,
        condition: Value<'context, 'module, C>,
        then_value: Value<'context, 'module, T>,
        else_value: Value<'context, 'module, T>,
        name: &str,
    ) -> Value<'context, 'module, T> {
        self.check_open("select");
        self.check_module("select", condition);
        self.check_module("select", then_value);
        self.check_module("select", else_value);
        // SAFETY: only reads them.
        if !unsafe { shim::lyftar_select_is_valid(condition.raw, then_value.raw, else_value.raw) } {
            bad_call(format_args!(
                "select: {} can't pick between {} and {}",
                condition.describe(),
                then_value.describe(),
                else_value.describe()
            ));
        }
        // SAFETY: just checked what LLVM requires.
        let result =
            unsafe { raw::LLVMBuildSelect(self.raw, condition.raw, then_value.raw, else_value.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: a select of two `T`s gives a `T`, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Reads the next variadic argument.
    ///
    /// # Panics
    /// If the list isn't a pointer, the type can't be read, it belongs to another module, or the block is already terminated
    /// (D3).
    #[track_caller]
    pub fn variable_argument<P: PointerOperand, T: TypeMarker>(
        &self,
        list: Value<'context, 'module, P>,
        argument_type: Type<'context, T>,
        name: &str,
    ) -> Value<'context, 'module, T> {
        self.check_open("variable_argument");
        self.check_module("variable_argument", list);
        P::check_pointer("variable_argument", list.to_dynamic());
        if !T::EXACT {
            check_first_class("variable_argument", argument_type);
        }
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildVAArg(self.raw, list.raw, argument_type.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: reading a `T` gives a `T`, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// A vector's element at an index.
    ///
    /// # Panics
    /// If LLVM's rules don't allow it, a value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn extract_element<V, I>(
        &self,
        vector: Value<'context, 'module, V>,
        index: Value<'context, 'module, I>,
        name: &str,
    ) -> Value<'context, 'module> {
        self.check_open("extract_element");
        self.check_module("extract_element", vector);
        self.check_module("extract_element", index);
        // SAFETY: only reads them.
        if !unsafe { shim::lyftar_extract_element_is_valid(vector.raw, index.raw) } {
            bad_call(format_args!("extract_element: can't take element {} of {}", index.describe(), vector.describe()));
        }
        // SAFETY: just checked what LLVM requires.
        let result = unsafe { raw::LLVMBuildExtractElement(self.raw, vector.raw, index.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: LLVM gave back a value live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// A vector with one element replaced.
    ///
    /// # Panics
    /// If LLVM's rules don't allow it, a value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn insert_element<V, E, I>(
        &self,
        vector: Value<'context, 'module, V>,
        element: Value<'context, 'module, E>,
        index: Value<'context, 'module, I>,
        name: &str,
    ) -> Value<'context, 'module, V> {
        self.check_open("insert_element");
        self.check_module("insert_element", vector);
        self.check_module("insert_element", element);
        self.check_module("insert_element", index);
        // SAFETY: only reads them.
        if !unsafe { shim::lyftar_insert_element_is_valid(vector.raw, element.raw, index.raw) } {
            bad_call(format_args!(
                "insert_element: can't put {} in {} at {}",
                element.describe(),
                vector.describe(),
                index.describe()
            ));
        }
        // SAFETY: just checked what LLVM requires.
        let result =
            unsafe { raw::LLVMBuildInsertElement(self.raw, vector.raw, element.raw, index.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the result has the vector's type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Two vectors shuffled by a mask.
    ///
    /// # Panics
    /// If LLVM's rules don't allow it, a value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn shuffle_vector(
        &self,
        left: Value<'context, 'module>,
        right: Value<'context, 'module>,
        mask: Value<'context, 'module>,
        name: &str,
    ) -> Value<'context, 'module> {
        self.check_open("shuffle_vector");
        self.check_module("shuffle_vector", left);
        self.check_module("shuffle_vector", right);
        self.check_module("shuffle_vector", mask);
        // SAFETY: only reads them.
        if !unsafe { shim::lyftar_shuffle_vector_is_valid(left.raw, right.raw, mask.raw) } {
            bad_call(format_args!(
                "shuffle_vector: can't shuffle {} and {} by {}",
                left.describe(),
                right.describe(),
                mask.describe()
            ));
        }
        // SAFETY: just checked what LLVM requires.
        let result = unsafe { raw::LLVMBuildShuffleVector(self.raw, left.raw, right.raw, mask.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: LLVM gave back a value live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// An aggregate's element at `index`.
    ///
    /// # Panics
    /// If it isn't an aggregate with such an element, it belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn extract_value<A>(
        &self,
        aggregate: Value<'context, 'module, A>,
        index: u32,
        name: &str,
    ) -> Value<'context, 'module> {
        self.check_open("extract_value");
        self.check_module("extract_value", aggregate);
        // SAFETY: only reads the type.
        let count = unsafe { shim::lyftar_aggregate_element_count(raw::LLVMTypeOf(aggregate.raw)) };
        if !checks::aggregate_index_in_bounds(u64::from(index), count) {
            bad_call(format_args!("extract_value: {} has no element {index}", aggregate.describe()));
        }
        // SAFETY: just checked what LLVM requires.
        let result = unsafe { raw::LLVMBuildExtractValue(self.raw, aggregate.raw, index, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: LLVM gave back a value live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// An aggregate with its element at `index` replaced.
    ///
    /// # Panics
    /// If it isn't an aggregate with such an element of that type, a value belongs to another module, or the block is
    /// already terminated (D3).
    #[track_caller]
    pub fn insert_value<A, E>(
        &self,
        aggregate: Value<'context, 'module, A>,
        element: Value<'context, 'module, E>,
        index: u32,
        name: &str,
    ) -> Value<'context, 'module, A> {
        self.check_open("insert_value");
        self.check_module("insert_value", aggregate);
        self.check_module("insert_value", element);
        // SAFETY: only reads the types.
        let valid = unsafe {
            shim::lyftar_aggregate_element_is(raw::LLVMTypeOf(aggregate.raw), index, raw::LLVMTypeOf(element.raw))
        };
        if !valid {
            bad_call(format_args!(
                "insert_value: {} can't go at element {index} of {}",
                element.describe(),
                aggregate.describe()
            ));
        }
        // SAFETY: just checked what LLVM requires.
        let result = unsafe { raw::LLVMBuildInsertValue(self.raw, aggregate.raw, element.raw, index, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the result has the aggregate's type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Stops a value from being `undef` or `poison`: `freeze`.
    ///
    /// # Panics
    /// If it belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn freeze<T>(&self, value: Value<'context, 'module, T>, name: &str) -> Value<'context, 'module, T> {
        self.check_open("freeze");
        self.check_module("freeze", value);
        // SAFETY: the checks above are what LLVM requires.
        let result = unsafe { raw::LLVMBuildFreeze(self.raw, value.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: the result has the value's type, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    // ---- Atomics ----

    /// A `fence`.
    ///
    /// # Panics
    /// If the ordering is too weak for a fence, or the block is already terminated (D3).
    #[track_caller]
    pub fn fence(&self, ordering: AtomicOrdering, single_thread: bool) -> Value<'context, 'module, FenceInstruction> {
        self.check_open("fence");
        check_fence_ordering(ordering);
        // SAFETY: the checks above are what LLVM requires.
        unsafe {
            Value::from_raw(raw::LLVMBuildFence(self.raw, ordering.to_raw(), i32::from(single_thread), EMPTY.as_ptr()))
        }
    }

    /// A `fence` in a synchronization scope.
    ///
    /// # Panics
    /// If the ordering is too weak for a fence, or the block is already terminated (D3).
    #[track_caller]
    pub fn fence_in_scope(
        &self,
        ordering: AtomicOrdering,
        scope: SynchronizationScope,
    ) -> Value<'context, 'module, FenceInstruction> {
        self.check_open("fence_in_scope");
        check_fence_ordering(ordering);
        // SAFETY: the checks above are what LLVM requires.
        unsafe { Value::from_raw(raw::LLVMBuildFenceSyncScope(self.raw, ordering.to_raw(), scope.0, EMPTY.as_ptr())) }
    }

    /// An atomic read, change and write: `atomicrmw`.
    ///
    /// # Panics
    /// If the ordering is too weak, the value's type doesn't suit the operation, the pointer isn't one, a value belongs to
    /// another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn atomic_read_modify_write<P: PointerOperand, T: ValueMarker>(
        &self,
        operation: AtomicReadModifyWriteOperation,
        pointer: Value<'context, 'module, P>,
        value: Value<'context, 'module, T>,
        ordering: AtomicOrdering,
        single_thread: bool,
    ) -> Value<'context, 'module, AtomicReadModifyWriteInstruction> {
        self.check_atomic_read_modify_write("atomic_read_modify_write", operation, pointer, value, ordering);
        // SAFETY: the checks above are what LLVM requires.
        unsafe {
            Value::from_raw(raw::LLVMBuildAtomicRMW(
                self.raw,
                operation.to_raw(),
                pointer.raw,
                value.raw,
                ordering.to_raw(),
                i32::from(single_thread),
            ))
        }
    }

    /// An atomic read, change and write in a synchronization scope.
    ///
    /// # Panics
    /// If the ordering is too weak, the value's type doesn't suit the operation, the pointer isn't one, a value belongs to
    /// another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn atomic_read_modify_write_in_scope<P: PointerOperand, T: ValueMarker>(
        &self,
        operation: AtomicReadModifyWriteOperation,
        pointer: Value<'context, 'module, P>,
        value: Value<'context, 'module, T>,
        ordering: AtomicOrdering,
        scope: SynchronizationScope,
    ) -> Value<'context, 'module, AtomicReadModifyWriteInstruction> {
        self.check_atomic_read_modify_write("atomic_read_modify_write_in_scope", operation, pointer, value, ordering);
        // SAFETY: the checks above are what LLVM requires.
        unsafe {
            Value::from_raw(raw::LLVMBuildAtomicRMWSyncScope(
                self.raw,
                operation.to_raw(),
                pointer.raw,
                value.raw,
                ordering.to_raw(),
                scope.0,
            ))
        }
    }

    #[track_caller]
    fn check_atomic_read_modify_write<P: PointerOperand, T: ValueMarker>(
        &self,
        name: &str,
        operation: AtomicReadModifyWriteOperation,
        pointer: Value<'context, 'module, P>,
        value: Value<'context, 'module, T>,
        ordering: AtomicOrdering,
    ) {
        self.check_open(name);
        self.check_module(name, pointer);
        self.check_module(name, value);
        P::check_pointer(name, pointer.to_dynamic());
        if matches!(ordering, AtomicOrdering::NotAtomic | AtomicOrdering::Unordered) {
            bad_call(format_args!("{name}: an atomicrmw can't be {ordering:?}"));
        }
        // SAFETY: only reads the type.
        let suits =
            unsafe { shim::lyftar_atomic_read_modify_write_is_valid(operation.to_raw(), raw::LLVMTypeOf(value.raw)) };
        if !suits {
            bad_call(format_args!("{name}: {operation:?} doesn't take {}", value.describe()));
        }
    }

    /// An atomic compare and exchange: `cmpxchg`.
    ///
    /// # Panics
    /// If an ordering is invalid, the types differ or can't be exchanged, the pointer isn't one, a value belongs to another
    /// module, or the block is already terminated (D3).
    #[track_caller]
    pub fn atomic_compare_exchange<P: PointerOperand, T: ValueMarker>(
        &self,
        pointer: Value<'context, 'module, P>,
        expected: Value<'context, 'module, T>,
        replacement: Value<'context, 'module, T>,
        success_ordering: AtomicOrdering,
        failure_ordering: AtomicOrdering,
        single_thread: bool,
    ) -> Value<'context, 'module, AtomicCompareExchangeInstruction> {
        self.check_atomic_compare_exchange(
            "atomic_compare_exchange",
            pointer,
            expected,
            replacement,
            success_ordering,
            failure_ordering,
        );
        // SAFETY: the checks above are what LLVM requires.
        unsafe {
            Value::from_raw(raw::LLVMBuildAtomicCmpXchg(
                self.raw,
                pointer.raw,
                expected.raw,
                replacement.raw,
                success_ordering.to_raw(),
                failure_ordering.to_raw(),
                i32::from(single_thread),
            ))
        }
    }

    /// An atomic compare and exchange in a synchronization scope.
    ///
    /// # Panics
    /// If an ordering is invalid, the types differ or can't be exchanged, the pointer isn't one, a value belongs to another
    /// module, or the block is already terminated (D3).
    #[track_caller]
    pub fn atomic_compare_exchange_in_scope<P: PointerOperand, T: ValueMarker>(
        &self,
        pointer: Value<'context, 'module, P>,
        expected: Value<'context, 'module, T>,
        replacement: Value<'context, 'module, T>,
        success_ordering: AtomicOrdering,
        failure_ordering: AtomicOrdering,
        scope: SynchronizationScope,
    ) -> Value<'context, 'module, AtomicCompareExchangeInstruction> {
        self.check_atomic_compare_exchange(
            "atomic_compare_exchange_in_scope",
            pointer,
            expected,
            replacement,
            success_ordering,
            failure_ordering,
        );
        // SAFETY: the checks above are what LLVM requires.
        unsafe {
            Value::from_raw(raw::LLVMBuildAtomicCmpXchgSyncScope(
                self.raw,
                pointer.raw,
                expected.raw,
                replacement.raw,
                success_ordering.to_raw(),
                failure_ordering.to_raw(),
                scope.0,
            ))
        }
    }

    #[track_caller]
    fn check_atomic_compare_exchange<P: PointerOperand, T: ValueMarker>(
        &self,
        name: &str,
        pointer: Value<'context, 'module, P>,
        expected: Value<'context, 'module, T>,
        replacement: Value<'context, 'module, T>,
        success_ordering: AtomicOrdering,
        failure_ordering: AtomicOrdering,
    ) {
        self.check_open(name);
        self.check_module(name, pointer);
        self.check_module(name, expected);
        self.check_module(name, replacement);
        P::check_pointer(name, pointer.to_dynamic());
        if !<T::LlvmType as TypeMarker>::EXACT {
            check_same_type(name, expected.raw, replacement.raw);
        }
        if matches!(success_ordering, AtomicOrdering::NotAtomic | AtomicOrdering::Unordered)
            || matches!(
                failure_ordering,
                AtomicOrdering::NotAtomic
                    | AtomicOrdering::Unordered
                    | AtomicOrdering::Release
                    | AtomicOrdering::AcquireRelease
            )
        {
            bad_call(format_args!("{name}: a cmpxchg can't be {success_ordering:?} then {failure_ordering:?}"));
        }
        // SAFETY: the value is live.
        let kind = unsafe { raw::LLVMGetTypeKind(raw::LLVMTypeOf(expected.raw)) };
        if !matches!(kind, raw::LLVMTypeKind::LLVMIntegerTypeKind | raw::LLVMTypeKind::LLVMPointerTypeKind) {
            bad_call(format_args!("{name}: {} isn't an integer or a pointer", expected.describe()));
        }
    }

    // ---- Terminators ----

    /// Returns from a function that returns nothing.
    ///
    /// # Panics
    /// If the function returns something, or the block is already terminated (D3).
    #[track_caller]
    pub fn return_void(&self) -> Value<'context, 'module, ReturnInstruction> {
        self.check_terminator_position("return_void");
        // SAFETY: the builder is in a function, so it has a type with a return type.
        let wanted = unsafe { raw::LLVMGetReturnType(raw::LLVMGlobalGetValueType(self.function.get())) };
        // SAFETY: that type is live.
        if unsafe { raw::LLVMGetTypeKind(wanted) } != raw::LLVMTypeKind::LLVMVoidTypeKind {
            bad_call(format_args!("return_void: this function returns a value"));
        }
        // SAFETY: the checks above are what LLVM requires.
        unsafe { Value::from_raw(raw::LLVMBuildRetVoid(self.raw)) }
    }

    /// Returns `value` from the function.
    ///
    /// # Panics
    /// If the function returns another type, the value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn return_value<T>(&self, value: Value<'context, 'module, T>) -> Value<'context, 'module, ReturnInstruction> {
        self.check_terminator_position("return_value");
        self.check_module("return_value", value);
        // SAFETY: the builder is in a function, so it has a type with a return type, and the value is live.
        let (wanted, given) = unsafe {
            (raw::LLVMGetReturnType(raw::LLVMGlobalGetValueType(self.function.get())), raw::LLVMTypeOf(value.raw))
        };
        if !checks::same_identity(wanted as usize, given as usize) {
            // SAFETY: the type is live, for as long as its context.
            let wanted: Type<'context> = unsafe { Type::from_raw(wanted) };
            bad_call(format_args!(
                "return_value: this function returns {}, not {}",
                wanted.describe(),
                value.describe()
            ));
        }
        // SAFETY: the checks above are what LLVM requires.
        unsafe { Value::from_raw(raw::LLVMBuildRet(self.raw, value.raw)) }
    }

    /// Returns several values, as the function's struct return type.
    ///
    /// # Panics
    /// If the function's return type doesn't match them, a value belongs to another module, or the block is already
    /// terminated (D3).
    #[track_caller]
    pub fn aggregate_return<T>(
        &self,
        values: &[Value<'context, 'module, T>],
    ) -> Value<'context, 'module, ReturnInstruction> {
        self.check_terminator_position("aggregate_return");
        self.check_values_module("aggregate_return", values);
        let count = length_as_u32(values.len(), "aggregate_return");
        // SAFETY: the builder is in a function, so it has a type with a return type.
        let wanted: Type<'context> =
            unsafe { Type::from_raw(raw::LLVMGetReturnType(raw::LLVMGlobalGetValueType(self.function.get()))) };
        // SAFETY: only reads them; `Value` has the layout of `LLVMValueRef`.
        let matches = unsafe { shim::lyftar_aggregate_matches(wanted.raw, values.as_ptr().cast(), count) };
        if !matches {
            bad_call(format_args!(
                "aggregate_return: this function returns {}, which {count} values don't make",
                wanted.describe()
            ));
        }
        // SAFETY: the checks above are what LLVM requires; it only reads the values.
        unsafe { Value::from_raw(raw::LLVMBuildAggregateRet(self.raw, values.as_ptr().cast_mut().cast(), count)) }
    }

    /// Branches to a block.
    ///
    /// # Panics
    /// If the block is in another function, or this block is already terminated (D3).
    #[track_caller]
    pub fn branch(
        &self,
        destination: BasicBlock<'context, 'module>,
    ) -> Value<'context, 'module, UnconditionalBranchInstruction> {
        self.check_terminator_position("branch");
        self.check_destination("branch", destination);
        // SAFETY: the checks above are what LLVM requires.
        unsafe { Value::from_raw(raw::LLVMBuildBr(self.raw, destination.raw)) }
    }

    /// Branches to one of two blocks by a condition.
    ///
    /// # Panics
    /// If the condition isn't an `i1` or belongs to another module, a block is in another function, or this block is already
    /// terminated (D3).
    #[track_caller]
    pub fn conditional_branch<C: ConditionOperand>(
        &self,
        condition: Value<'context, 'module, C>,
        then_destination: BasicBlock<'context, 'module>,
        else_destination: BasicBlock<'context, 'module>,
    ) -> Value<'context, 'module, ConditionalBranchInstruction> {
        self.check_terminator_position("conditional_branch");
        self.check_module("conditional_branch", condition);
        C::check_condition("conditional_branch", condition.to_dynamic());
        self.check_destination("conditional_branch", then_destination);
        self.check_destination("conditional_branch", else_destination);
        // SAFETY: the checks above are what LLVM requires.
        unsafe {
            Value::from_raw(raw::LLVMBuildCondBr(self.raw, condition.raw, then_destination.raw, else_destination.raw))
        }
    }

    /// A `switch`, to add cases to afterwards with [`Value::add_case`].
    ///
    /// # Panics
    /// If the value isn't an integer or belongs to another module, the block is in another function, or this block is
    /// already terminated (D3).
    #[track_caller]
    pub fn switch<T: IntegerOperand>(
        &self,
        value: Value<'context, 'module, T>,
        default_destination: BasicBlock<'context, 'module>,
        case_count_hint: u32,
    ) -> Value<'context, 'module, SwitchInstruction> {
        self.check_terminator_position("switch");
        self.check_module("switch", value);
        check_single_integer("switch", value.raw);
        self.check_destination("switch", default_destination);
        // SAFETY: the checks above are what LLVM requires.
        unsafe { Value::from_raw(raw::LLVMBuildSwitch(self.raw, value.raw, default_destination.raw, case_count_hint)) }
    }

    /// An `indirectbr`, to add destinations to afterwards with [`Value::add_destination`].
    ///
    /// # Panics
    /// If the address isn't a pointer or belongs to another module, or this block is already terminated (D3).
    #[track_caller]
    pub fn indirect_branch<P: PointerOperand>(
        &self,
        address: Value<'context, 'module, P>,
        destination_count_hint: u32,
    ) -> Value<'context, 'module, IndirectBranchInstruction> {
        self.check_terminator_position("indirect_branch");
        self.check_module("indirect_branch", address);
        P::check_pointer("indirect_branch", address.to_dynamic());
        // SAFETY: the checks above are what LLVM requires.
        unsafe { Value::from_raw(raw::LLVMBuildIndirectBr(self.raw, address.raw, destination_count_hint)) }
    }

    /// A `callbr`: calls inline assembly that can jump elsewhere.
    ///
    /// # Panics
    /// If the arguments don't fit, a value belongs to another module, a block is in another function, or this block is
    /// already terminated (D3).
    #[track_caller]
    #[allow(clippy::too_many_arguments)]
    pub fn call_branch<P: PointerOperand, A>(
        &self,
        function_type: Type<'context>,
        callee: Value<'context, 'module, P>,
        default_destination: BasicBlock<'context, 'module>,
        indirect_destinations: &[BasicBlock<'context, 'module>],
        arguments: &[Value<'context, 'module, A>],
        bundles: &[OperandBundle<'context, 'module>],
        name: &str,
    ) -> Value<'context, 'module, CallBranchInstruction> {
        self.check_terminator_position("call_branch");
        self.check_module("call_branch", callee);
        P::check_pointer("call_branch", callee.to_dynamic());
        let count = self.check_call("call_branch", function_type, arguments);
        self.check_bundles("call_branch", bundles);
        self.check_destination("call_branch", default_destination);
        for destination in indirect_destinations {
            self.check_destination("call_branch", *destination);
        }
        // SAFETY: the checks above are what LLVM requires; it only reads the lists, and `BasicBlock` and `OperandBundle` have the
        // layout of their raw pointers.
        let result = unsafe {
            raw::LLVMBuildCallBr(
                self.raw,
                function_type.raw,
                callee.raw,
                default_destination.raw,
                indirect_destinations.as_ptr().cast_mut().cast(),
                length_as_u32(indirect_destinations.len(), "call_branch"),
                arguments.as_ptr().cast_mut().cast(),
                count,
                bundles.as_ptr().cast_mut().cast(),
                length_as_u32(bundles.len(), "call_branch"),
                EMPTY.as_ptr(),
            )
        };
        set_name(result, name);
        // SAFETY: LLVM just built it, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Calls a function that may unwind: `invoke`.
    ///
    /// # Panics
    /// If the arguments don't fit, a value belongs to another module, a block is in another function, or this block is
    /// already terminated (D3).
    #[track_caller]
    pub fn invoke<P: PointerOperand, A>(
        &self,
        function_type: Type<'context>,
        callee: Value<'context, 'module, P>,
        arguments: &[Value<'context, 'module, A>],
        normal_destination: BasicBlock<'context, 'module>,
        unwind_destination: BasicBlock<'context, 'module>,
        name: &str,
    ) -> Value<'context, 'module, InvokeInstruction> {
        self.check_terminator_position("invoke");
        self.check_module("invoke", callee);
        P::check_pointer("invoke", callee.to_dynamic());
        let count = self.check_call("invoke", function_type, arguments);
        self.check_destination("invoke", normal_destination);
        self.check_destination("invoke", unwind_destination);
        // SAFETY: the checks above are what LLVM requires; it only reads the arguments.
        let result = unsafe {
            raw::LLVMBuildInvoke2(
                self.raw,
                function_type.raw,
                callee.raw,
                arguments.as_ptr().cast_mut().cast(),
                count,
                normal_destination.raw,
                unwind_destination.raw,
                EMPTY.as_ptr(),
            )
        };
        set_name(result, name);
        // SAFETY: LLVM just built it, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// As [`Builder::invoke`], with operand bundles.
    ///
    /// # Panics
    /// If the arguments don't fit, a value belongs to another module, a block is in another function, or this block is
    /// already terminated (D3).
    #[track_caller]
    #[allow(clippy::too_many_arguments)]
    pub fn invoke_with_operand_bundles<P: PointerOperand, A>(
        &self,
        function_type: Type<'context>,
        callee: Value<'context, 'module, P>,
        arguments: &[Value<'context, 'module, A>],
        normal_destination: BasicBlock<'context, 'module>,
        unwind_destination: BasicBlock<'context, 'module>,
        bundles: &[OperandBundle<'context, 'module>],
        name: &str,
    ) -> Value<'context, 'module, InvokeInstruction> {
        self.check_terminator_position("invoke_with_operand_bundles");
        self.check_module("invoke_with_operand_bundles", callee);
        P::check_pointer("invoke_with_operand_bundles", callee.to_dynamic());
        let count = self.check_call("invoke_with_operand_bundles", function_type, arguments);
        self.check_bundles("invoke_with_operand_bundles", bundles);
        self.check_destination("invoke_with_operand_bundles", normal_destination);
        self.check_destination("invoke_with_operand_bundles", unwind_destination);
        // SAFETY: the checks above are what LLVM requires; it only reads the lists.
        let result = unsafe {
            raw::LLVMBuildInvokeWithOperandBundles(
                self.raw,
                function_type.raw,
                callee.raw,
                arguments.as_ptr().cast_mut().cast(),
                count,
                normal_destination.raw,
                unwind_destination.raw,
                bundles.as_ptr().cast_mut().cast(),
                length_as_u32(bundles.len(), "invoke_with_operand_bundles"),
                EMPTY.as_ptr(),
            )
        };
        set_name(result, name);
        // SAFETY: LLVM just built it, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Marks that control never reaches here.
    ///
    /// # Panics
    /// If this block is already terminated (D3).
    #[track_caller]
    pub fn unreachable(&self) -> Value<'context, 'module, UnreachableInstruction> {
        self.check_terminator_position("unreachable");
        // SAFETY: the check above is what LLVM requires.
        unsafe { Value::from_raw(raw::LLVMBuildUnreachable(self.raw)) }
    }

    /// Continues unwinding: `resume`.
    ///
    /// # Panics
    /// If it belongs to another module, or this block is already terminated (D3).
    #[track_caller]
    pub fn resume<T>(&self, exception: Value<'context, 'module, T>) -> Value<'context, 'module, ResumeInstruction> {
        self.check_terminator_position("resume");
        self.check_module("resume", exception);
        // SAFETY: the checks above are what LLVM requires.
        unsafe { Value::from_raw(raw::LLVMBuildResume(self.raw, exception.raw)) }
    }

    /// A `landingpad`, with its function's personality if given, to add clauses to afterwards.
    ///
    /// # Panics
    /// If the personality belongs to another module or isn't a constant pointer, or the block is already terminated (D3).
    #[track_caller]
    pub fn landing_pad<T>(
        &self,
        result_type: Type<'context, T>,
        personality: Option<Value<'context, 'module, Function>>,
        clause_count_hint: u32,
        name: &str,
    ) -> Value<'context, 'module, LandingPadInstruction> {
        self.check_open("landing_pad");
        let raw_personality = match personality {
            Some(function) => {
                self.check_module("landing_pad", function);
                function.raw
            }
            None => std::ptr::null_mut(),
        };
        // SAFETY: the checks above are what LLVM requires; a null personality leaves the function's as it is.
        let result = unsafe {
            raw::LLVMBuildLandingPad(self.raw, result_type.raw, raw_personality, clause_count_hint, EMPTY.as_ptr())
        };
        set_name(result, name);
        // SAFETY: LLVM just built it, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Leaves a cleanup: `cleanupret`.
    ///
    /// # Panics
    /// If a value belongs to another module, the block is in another function, or this block is already terminated (D3).
    #[track_caller]
    pub fn cleanup_return(
        &self,
        cleanup_pad: Value<'context, 'module, CleanupPadInstruction>,
        unwind_destination: Option<BasicBlock<'context, 'module>>,
    ) -> Value<'context, 'module, CleanupReturnInstruction> {
        self.check_terminator_position("cleanup_return");
        self.check_module("cleanup_return", cleanup_pad);
        let raw_block = match unwind_destination {
            Some(block) => {
                self.check_destination("cleanup_return", block);
                block.raw
            }
            None => std::ptr::null_mut(),
        };
        // SAFETY: the checks above are what LLVM requires; a null block unwinds to the caller.
        unsafe { Value::from_raw(raw::LLVMBuildCleanupRet(self.raw, cleanup_pad.raw, raw_block)) }
    }

    /// Leaves a catch: `catchret`.
    ///
    /// # Panics
    /// If a value belongs to another module, the block is in another function, or this block is already terminated (D3).
    #[track_caller]
    pub fn catch_return(
        &self,
        catch_pad: Value<'context, 'module, CatchPadInstruction>,
        destination: BasicBlock<'context, 'module>,
    ) -> Value<'context, 'module, CatchReturnInstruction> {
        self.check_terminator_position("catch_return");
        self.check_module("catch_return", catch_pad);
        self.check_destination("catch_return", destination);
        // SAFETY: the checks above are what LLVM requires.
        unsafe { Value::from_raw(raw::LLVMBuildCatchRet(self.raw, catch_pad.raw, destination.raw)) }
    }

    /// A `catchpad` under a `catchswitch`.
    ///
    /// # Panics
    /// If a value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn catch_pad<A>(
        &self,
        catch_switch: Value<'context, 'module, CatchSwitchInstruction>,
        arguments: &[Value<'context, 'module, A>],
        name: &str,
    ) -> Value<'context, 'module, CatchPadInstruction> {
        self.check_open("catch_pad");
        self.check_module("catch_pad", catch_switch);
        self.check_values_module("catch_pad", arguments);
        let count = length_as_u32(arguments.len(), "catch_pad");
        // SAFETY: the checks above are what LLVM requires; it only reads the arguments.
        let result = unsafe {
            raw::LLVMBuildCatchPad(
                self.raw,
                catch_switch.raw,
                arguments.as_ptr().cast_mut().cast(),
                count,
                EMPTY.as_ptr(),
            )
        };
        set_name(result, name);
        // SAFETY: LLVM just built it, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// A `cleanuppad`, inside another pad if given.
    ///
    /// # Panics
    /// If a value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn cleanup_pad<A, P>(
        &self,
        parent_pad: Option<Value<'context, 'module, P>>,
        arguments: &[Value<'context, 'module, A>],
        name: &str,
    ) -> Value<'context, 'module, CleanupPadInstruction> {
        self.check_open("cleanup_pad");
        self.check_values_module("cleanup_pad", arguments);
        let count = length_as_u32(arguments.len(), "cleanup_pad");
        let raw_parent = match parent_pad {
            Some(pad) => {
                self.check_module("cleanup_pad", pad);
                pad.raw
            }
            // LLVM wants the `none` token for no parent.
            // SAFETY: the builder's context is live.
            None => unsafe { shim::lyftar_token_none(raw::LLVMGetBuilderContext(self.raw)) },
        };
        // SAFETY: the checks above are what LLVM requires; it only reads the arguments.
        let result = unsafe {
            raw::LLVMBuildCleanupPad(self.raw, raw_parent, arguments.as_ptr().cast_mut().cast(), count, EMPTY.as_ptr())
        };
        set_name(result, name);
        // SAFETY: LLVM just built it, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// A `catchswitch`, to add handlers to afterwards with [`Value::add_handler`].
    ///
    /// # Panics
    /// If a value belongs to another module, a block is in another function, or this block is already terminated (D3).
    #[track_caller]
    pub fn catch_switch<P>(
        &self,
        parent_pad: Option<Value<'context, 'module, P>>,
        unwind_destination: Option<BasicBlock<'context, 'module>>,
        handler_count_hint: u32,
        name: &str,
    ) -> Value<'context, 'module, CatchSwitchInstruction> {
        self.check_terminator_position("catch_switch");
        let raw_parent = match parent_pad {
            Some(pad) => {
                self.check_module("catch_switch", pad);
                pad.raw
            }
            // LLVM wants the `none` token for no parent.
            // SAFETY: the builder's context is live.
            None => unsafe { shim::lyftar_token_none(raw::LLVMGetBuilderContext(self.raw)) },
        };
        let raw_unwind = match unwind_destination {
            Some(block) => {
                self.check_destination("catch_switch", block);
                block.raw
            }
            None => std::ptr::null_mut(),
        };
        // SAFETY: the checks above are what LLVM requires; a null block unwinds to the caller.
        let result =
            unsafe { raw::LLVMBuildCatchSwitch(self.raw, raw_parent, raw_unwind, handler_count_hint, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: LLVM just built it, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    // ---- Moving what exists ----

    /// Puts a copy of `instruction` here, with `name`.
    ///
    /// # Panics
    /// If it belongs to another module, or it would go after a terminator (D3).
    #[track_caller]
    pub fn insert_clone<T: InstructionMarker>(
        &self,
        instruction: Value<'context, 'module, T>,
        name: &str,
    ) -> Value<'context, 'module, T> {
        self.check_open("insert_clone");
        self.check_module("insert_clone", instruction);
        // SAFETY: the instruction is live; the clone goes in the block right away, which then owns it.
        unsafe {
            let clone = raw::LLVMInstructionClone(instruction.raw);
            raw::LLVMInsertIntoBuilder(self.raw, clone);
            set_name(clone, name);
            Value::from_raw(clone)
        }
    }

    /// Moves `instruction` here.
    ///
    /// # Panics
    /// If it belongs to another module, or it would go after a terminator (D3).
    #[track_caller]
    pub fn move_instruction_here<T: InstructionMarker>(&self, instruction: Value<'context, 'module, T>) {
        self.check_open("move_instruction_here");
        self.check_module("move_instruction_here", instruction);
        // SAFETY: both are live, and the shim handles a builder placed right before the instruction.
        unsafe { shim::lyftar_builder_move_here(self.raw, instruction.raw) };
    }
}

/// What the shim gives back when a call's arguments fit.
const CALL_IS_VALID: u32 = u32::MAX;
/// What the shim gives back when a call has the wrong number of arguments.
const CALL_HAS_WRONG_COUNT: u32 = u32::MAX - 1;

fn same_shape(left: raw::LLVMTypeRef, right: raw::LLVMTypeRef) -> bool {
    use raw::LLVMTypeKind as K;
    // SAFETY: both are live types, and a size is only asked of vectors.
    unsafe {
        let (left_kind, right_kind) = (raw::LLVMGetTypeKind(left), raw::LLVMGetTypeKind(right));
        let left_is_vector = matches!(left_kind, K::LLVMVectorTypeKind | K::LLVMScalableVectorTypeKind);
        if left_is_vector {
            left_kind == right_kind && raw::LLVMGetVectorSize(left) == raw::LLVMGetVectorSize(right)
        } else {
            !matches!(right_kind, K::LLVMVectorTypeKind | K::LLVMScalableVectorTypeKind)
        }
    }
}

fn set_global_name(value: raw::LLVMValueRef, name: &str) {
    if !name.is_empty() {
        // SAFETY: the value is live, and LLVM reads `name.len()` bytes of `name`.
        unsafe { raw::LLVMSetValueName2(value, name.as_ptr().cast(), name.len()) };
    }
}

#[track_caller]
fn check_sized<T>(operation: &str, checked: Type<'_, T>) {
    if !checked.is_sized() {
        bad_call(format_args!("{operation}: {} has no size", checked.describe()));
    }
}

#[track_caller]
fn check_optional_alignment(operation: &str, alignment_in_bytes: u32) {
    if alignment_in_bytes != 0 && !checks::is_power_of_two(alignment_in_bytes) {
        bad_call(format_args!("{operation}: {alignment_in_bytes} is not zero or a power of two"));
    }
}

#[track_caller]
fn check_loadable<T>(operation: &str, checked: Type<'_, T>) {
    // SAFETY: only reads the type.
    if !unsafe { shim::lyftar_type_is_loadable(checked.raw) } {
        bad_call(format_args!("{operation}: {} can't be loaded or stored", checked.describe()));
    }
}

#[track_caller]
fn check_first_class<T>(operation: &str, checked: Type<'_, T>) {
    // SAFETY: only reads the type.
    if !unsafe { shim::lyftar_type_is_first_class(checked.raw) } {
        bad_call(format_args!("{operation}: {} isn't a type a value can have here", checked.describe()));
    }
}

#[track_caller]
fn check_integer_or_pointer(operation: &str, value: raw::LLVMValueRef) {
    use raw::LLVMTypeKind as K;
    // SAFETY: the value came from a live `Value`.
    let kind = unsafe { raw::LLVMGetTypeKind(raw::LLVMTypeOf(value)) };
    if !matches!(kind, K::LLVMIntegerTypeKind | K::LLVMPointerTypeKind) {
        bad_operand(operation, value, "an integer or a pointer");
    }
}

#[track_caller]
fn check_single_integer(operation: &str, value: raw::LLVMValueRef) {
    // SAFETY: the value came from a live `Value`.
    if unsafe { raw::LLVMGetTypeKind(raw::LLVMTypeOf(value)) } != raw::LLVMTypeKind::LLVMIntegerTypeKind {
        bad_operand(operation, value, "a single integer");
    }
}

#[track_caller]
fn check_fence_ordering(ordering: AtomicOrdering) {
    if matches!(ordering, AtomicOrdering::NotAtomic | AtomicOrdering::Unordered | AtomicOrdering::Monotonic) {
        bad_call(format_args!("fence: a fence can't be {ordering:?}"));
    }
}
