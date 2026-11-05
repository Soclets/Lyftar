//! LLVM values (D4, D11).
//!
//! A [`Value`] says as much as the code knows: `Value<'context, 'module, Integer<Bits<64>>>` is checked at `cargo build`,
//! `Value<'context, 'module>` when the line runs, and a marker such as [`LoadInstruction`] says what kind of LLVM value it
//! is, which `value.as_kind::<LoadInstruction>()` asks about (D4).

use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::basic_block::BasicBlock;
use crate::checks;
use crate::context::Context;
use crate::enums::ValueCategory;
use crate::module::Module;
use crate::raw;
use crate::shim;
use crate::text::{from_owned_message, from_pointer_and_length};
use crate::types::{Dynamic, Invariant, ReturnType, Sealed, Type, TypeMarker, TypeTuple, length_as_u32};

/// A value: as specific as the code knows (D4), belonging to its context (invariant, D10) and borrowing that context and its
/// module (covariant, D11, D42), so that holding one across a delete or a pass run on any module doesn't compile. Plain data
/// borrows nothing, carrying `'static`, and survives those (D41).
#[repr(transparent)]
pub struct Value<'context, 'module, T = Dynamic> {
    pub(crate) raw: raw::LLVMValueRef,
    marker: ValueLifetimes<'context, 'module, T>,
}

/// What a `Value` carries with no data: an invariant context, a covariant module borrow, and what it knows.
type ValueLifetimes<'context, 'module, T> = PhantomData<(Invariant<'context>, &'module (), fn() -> T)>;

impl<T> Clone for Value<'_, '_, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Value<'_, '_, T> {}
impl<T, U> PartialEq<Value<'_, '_, U>> for Value<'_, '_, T> {
    fn eq(&self, other: &Value<'_, '_, U>) -> bool {
        self.raw == other.raw
    }
}
impl<T> Eq for Value<'_, '_, T> {}
impl<T> std::hash::Hash for Value<'_, '_, T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}
impl<T> std::fmt::Debug for Value<'_, '_, T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.print_to_string())
    }
}

/// What Rust knows about a value (D4).
pub trait ValueMarker: Sealed + 'static {
    /// What is known about this value's LLVM type.
    type LlvmType: TypeMarker;

    /// Whether `raw` is such a value.
    ///
    /// # Safety
    /// `raw` must be a live LLVM value.
    #[doc(hidden)]
    unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool;
}

impl<T: TypeMarker> ValueMarker for T {
    type LlvmType = T;

    unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool {
        // SAFETY: the caller promises a live value, and a live value has a live type.
        unsafe { T::accepts_type(raw::LLVMTypeOf(raw)) }
    }
}

/// A value that uses others, so it has operands.
pub trait UserMarker: ValueMarker {}
/// A constant.
pub trait ConstantMarker: UserMarker {}
/// A global value: a function, a global variable, an alias or an indirect function.
pub trait GlobalValueMarker: ConstantMarker {}
/// A global object: a function, a global variable or an indirect function.
pub trait GlobalObjectMarker: GlobalValueMarker {}
/// An instruction.
pub trait InstructionMarker: UserMarker {}
/// An instruction that calls something: a call, an invoke or a call branch.
pub trait CallBaseMarker: InstructionMarker {}
/// Something with an alignment: a load, store, stack allocation, atomic operation or global object.
pub trait AlignedMarker: ValueMarker {}
/// A load or store, or an atomic operation that can be marked volatile.
pub trait VolatileMarker: InstructionMarker {}
/// An instruction with a syncronization scope.
pub trait AtomicMarker: InstructionMarker {}
/// A terminator.
pub trait TerminatorMarker: InstructionMarker {}

/// A function, as a value (D4, D35). Its LLVM type is a pointer, in the address space the target says (D31).
///
/// Plain `Function` is dynamic. A known signature is `Function<R, (A, B)>`, as in `Function<Void, ()>`, so a call's arguments
/// and result are checked at `cargo build`, up to 12 arguments; longer ones use the dynamic form (D33).
#[derive(Clone, Copy, Debug)]
// trace: D33
pub struct Function<Result = Dynamic, Parameters = Dynamic>(PhantomData<fn() -> (Result, Parameters)>);

/// A variadic function with a known signature, `VariadicFunction<R, (A, B)>`, where only the fixed arguments are checked
/// at `cargo build` (D33).
#[derive(Clone, Copy, Debug)]
pub struct VariadicFunction<Result, Parameters>(PhantomData<fn() -> (Result, Parameters)>);

/// What says a value is a function: [`Function`] and [`VariadicFunction`].
pub trait FunctionMarker: GlobalObjectMarker + AlignedMarker {}

/// A function whose signature is written into the Rust code, for [`Module::add_specific_function`](crate::Module::add_specific_function).
pub trait SpecificFunction: FunctionMarker {
    #[doc(hidden)]
    fn raw_function_type(context: &Context<'_>) -> raw::LLVMTypeRef;
}

/// Whether `raw` is a function of the type `T` says.
///
/// # Safety
/// `raw` must be a live LLVM value.
unsafe fn is_function_of<T: SpecificFunction>(raw: raw::LLVMValueRef) -> bool {
    // SAFETY: the caller promises a live value, and the type of a function is live with it. LLVM uniques function types.
    unsafe {
        if raw::LLVMIsAFunction(raw).is_null() {
            return false;
        }
        let own_type = raw::LLVMGlobalGetValueType(raw);
        own_type == T::raw_function_type(&Context::from_raw(raw::LLVMGetTypeContext(own_type)))
    }
}

/// Gives a function marker what every function marker has.
macro_rules! function_marker {
    ([$($generics:tt)*] $marker:ty) => {
        impl<$($generics)*> UserMarker for $marker {}
        impl<$($generics)*> ConstantMarker for $marker {}
        impl<$($generics)*> GlobalValueMarker for $marker {}
        impl<$($generics)*> GlobalObjectMarker for $marker {}
        impl<$($generics)*> AlignedMarker for $marker {}
        impl<$($generics)*> FunctionMarker for $marker {}
    };
}

impl Sealed for Function {}
impl ValueMarker for Function {
    type LlvmType = Dynamic;
    unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool {
        // SAFETY: the caller promises a live value.
        unsafe { !raw::LLVMIsAFunction(raw).is_null() }
    }
}
function_marker!([] Function);

/// The function type for a return type and parameters.
fn raw_function_type<R: ReturnType, P: TypeTuple>(context: &Context<'_>, is_variadic: bool) -> raw::LLVMTypeRef {
    let parameters = P::raw_types(context);
    let parameters = parameters.as_ref();
    let count = length_as_u32(parameters.len(), "add_specific_function");
    // SAFETY: the bounds on the markers make each type valid here, and LLVM only reads the parameters.
    unsafe { raw::LLVMFunctionType(R::get_raw(context), parameters.as_ptr().cast_mut(), count, i32::from(is_variadic)) }
}

impl<R: ReturnType, P: TypeTuple> Sealed for Function<R, P> {}
impl<R: ReturnType, P: TypeTuple> ValueMarker for Function<R, P> {
    type LlvmType = Dynamic;
    unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool {
        // SAFETY: the caller promises a live value.
        unsafe { is_function_of::<Self>(raw) }
    }
}
function_marker!([R: ReturnType, P: TypeTuple] Function<R, P>);
impl<R: ReturnType, P: TypeTuple> SpecificFunction for Function<R, P> {
    fn raw_function_type(context: &Context<'_>) -> raw::LLVMTypeRef {
        raw_function_type::<R, P>(context, false)
    }
}

impl<R: ReturnType, P: TypeTuple> Sealed for VariadicFunction<R, P> {}
impl<R: ReturnType, P: TypeTuple> ValueMarker for VariadicFunction<R, P> {
    type LlvmType = Dynamic;
    unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool {
        // SAFETY: the caller promises a live value.
        unsafe { is_function_of::<Self>(raw) }
    }
}
function_marker!([R: ReturnType, P: TypeTuple] VariadicFunction<R, P>);
impl<R: ReturnType, P: TypeTuple> SpecificFunction for VariadicFunction<R, P> {
    fn raw_function_type(context: &Context<'_>) -> raw::LLVMTypeRef {
        raw_function_type::<R, P>(context, true)
    }
}

/// Declares the markers for LLVM's value classes, one per `LLVMIsA*` (D4), each spelled out (D5).
macro_rules! value_markers {
    ($($(#[$meta:meta])* $name:ident = $is_a:ident as $llvm_type:ty : [$($marker:ident),*];)*) => {
        $(
            $(#[$meta])*
            #[derive(Clone, Copy, Debug)]
            pub struct $name;
            impl Sealed for $name {}
            impl ValueMarker for $name {
                type LlvmType = $llvm_type;
                unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool {
                    // SAFETY: the caller promises a live value.
                    unsafe { !raw::$is_a(raw).is_null() }
                }
            }
            $(impl $marker for $name {})*
        )*
    };
}

value_markers! {
    /// A function's argument.
    Argument = LLVMIsAArgument as Dynamic: [];
    /// A basic block, as a value. See [`BasicBlock`] for working with one.
    BasicBlockAsValue = LLVMIsABasicBlock as Dynamic: [];
    /// Inline assembly.
    InlineAssembly = LLVMIsAInlineAsm as Dynamic: [];
    /// Any value that uses others.
    User = LLVMIsAUser as Dynamic: [UserMarker];
    /// Any constant.
    Constant = LLVMIsAConstant as Dynamic: [UserMarker, ConstantMarker];
    /// A block's address.
    BlockAddress = LLVMIsABlockAddress as Dynamic: [UserMarker, ConstantMarker];
    /// A zero aggregate.
    ConstantAggregateZero = LLVMIsAConstantAggregateZero as Dynamic: [UserMarker, ConstantMarker];
    /// A constant array.
    ConstantArray = LLVMIsAConstantArray as Dynamic: [UserMarker, ConstantMarker];
    /// A constant array or vector stored as plain data.
    ConstantDataSequential = LLVMIsAConstantDataSequential as Dynamic: [UserMarker, ConstantMarker];
    /// A constant array stored as plain data.
    ConstantDataArray = LLVMIsAConstantDataArray as Dynamic: [UserMarker, ConstantMarker];
    /// A constant vector stored as plain data.
    ConstantDataVector = LLVMIsAConstantDataVector as Dynamic: [UserMarker, ConstantMarker];
    /// A constant expression.
    ConstantExpression = LLVMIsAConstantExpr as Dynamic: [UserMarker, ConstantMarker];
    /// A constant float.
    ConstantFloat = LLVMIsAConstantFP as Dynamic: [UserMarker, ConstantMarker];
    /// A constant integer.
    ConstantInteger = LLVMIsAConstantInt as Dynamic: [UserMarker, ConstantMarker];
    /// A constant byte.
    ConstantByte = LLVMIsAConstantByte as Dynamic: [UserMarker, ConstantMarker];
    /// A constant null pointer.
    ConstantPointerNull = LLVMIsAConstantPointerNull as Dynamic: [UserMarker, ConstantMarker];
    /// A constant struct.
    ConstantStruct = LLVMIsAConstantStruct as Dynamic: [UserMarker, ConstantMarker];
    /// The constant `none` token.
    ConstantTokenNone = LLVMIsAConstantTokenNone as Dynamic: [UserMarker, ConstantMarker];
    /// A constant vector.
    ConstantVector = LLVMIsAConstantVector as Dynamic: [UserMarker, ConstantMarker];
    /// A constant authenticated pointer.
    ConstantPointerAuthentication = LLVMIsAConstantPtrAuth as Dynamic: [UserMarker, ConstantMarker];
    /// Any global value.
    GlobalValue = LLVMIsAGlobalValue as Dynamic: [UserMarker, ConstantMarker, GlobalValueMarker];
    /// A global alias.
    GlobalAlias = LLVMIsAGlobalAlias as Dynamic: [UserMarker, ConstantMarker, GlobalValueMarker];
    /// Any global object.
    GlobalObject = LLVMIsAGlobalObject as Dynamic:
        [UserMarker, ConstantMarker, GlobalValueMarker, GlobalObjectMarker, AlignedMarker];
    /// A global variable.
    GlobalVariable = LLVMIsAGlobalVariable as Dynamic:
        [UserMarker, ConstantMarker, GlobalValueMarker, GlobalObjectMarker, AlignedMarker];
    /// A global indirect function.
    GlobalIndirectFunction = LLVMIsAGlobalIFunc as Dynamic:
        [UserMarker, ConstantMarker, GlobalValueMarker, GlobalObjectMarker, AlignedMarker];
    /// `undef`.
    UndefinedValue = LLVMIsAUndefValue as Dynamic: [UserMarker, ConstantMarker];
    /// `poison`.
    PoisonValue = LLVMIsAPoisonValue as Dynamic: [UserMarker, ConstantMarker];
    /// Any instruction.
    Instruction = LLVMIsAInstruction as Dynamic: [UserMarker, InstructionMarker];
    /// An operator with one operand, such as `fneg`.
    UnaryOperator = LLVMIsAUnaryOperator as Dynamic: [UserMarker, InstructionMarker];
    /// An operator with two operands, such as `add`.
    BinaryOperator = LLVMIsABinaryOperator as Dynamic: [UserMarker, InstructionMarker];
    /// A `call`.
    CallInstruction = LLVMIsACallInst as Dynamic: [UserMarker, InstructionMarker, CallBaseMarker];
    /// A call of an intrinsic.
    IntrinsicInstruction = LLVMIsAIntrinsicInst as Dynamic: [UserMarker, InstructionMarker, CallBaseMarker];
    /// A call of a debug intrinsic.
    DebugInfoIntrinsic = LLVMIsADbgInfoIntrinsic as Dynamic: [UserMarker, InstructionMarker, CallBaseMarker];
    /// A call of a debug variable intrinsic.
    DebugVariableIntrinsic = LLVMIsADbgVariableIntrinsic as Dynamic: [UserMarker, InstructionMarker, CallBaseMarker];
    /// A call of `llvm.dbg.declare`.
    DebugDeclareInstruction = LLVMIsADbgDeclareInst as Dynamic: [UserMarker, InstructionMarker, CallBaseMarker];
    /// A call of `llvm.dbg.label`.
    DebugLabelInstruction = LLVMIsADbgLabelInst as Dynamic: [UserMarker, InstructionMarker, CallBaseMarker];
    /// A call of `llvm.memcpy`, `llvm.memmove` or `llvm.memset`.
    MemoryIntrinsic = LLVMIsAMemIntrinsic as Dynamic: [UserMarker, InstructionMarker, CallBaseMarker];
    /// A call of `llvm.memcpy`.
    MemoryCopyInstruction = LLVMIsAMemCpyInst as Dynamic: [UserMarker, InstructionMarker, CallBaseMarker];
    /// A call of `llvm.memmove`.
    MemoryMoveInstruction = LLVMIsAMemMoveInst as Dynamic: [UserMarker, InstructionMarker, CallBaseMarker];
    /// A call of `llvm.memset`.
    MemorySetInstruction = LLVMIsAMemSetInst as Dynamic: [UserMarker, InstructionMarker, CallBaseMarker];
    /// An `icmp` or an `fcmp`.
    CompareInstruction = LLVMIsACmpInst as Dynamic: [UserMarker, InstructionMarker];
    /// An `fcmp`.
    FloatCompareInstruction = LLVMIsAFCmpInst as Dynamic: [UserMarker, InstructionMarker];
    /// An `icmp`.
    IntegerCompareInstruction = LLVMIsAICmpInst as Dynamic: [UserMarker, InstructionMarker];
    /// An `extractelement`.
    ExtractElementInstruction = LLVMIsAExtractElementInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `getelementptr`.
    GetElementPointerInstruction = LLVMIsAGetElementPtrInst as Dynamic: [UserMarker, InstructionMarker];
    /// An `insertelement`.
    InsertElementInstruction = LLVMIsAInsertElementInst as Dynamic: [UserMarker, InstructionMarker];
    /// An `insertvalue`.
    InsertValueInstruction = LLVMIsAInsertValueInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `landingpad`.
    LandingPadInstruction = LLVMIsALandingPadInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `select`.
    SelectInstruction = LLVMIsASelectInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `shufflevector`.
    ShuffleVectorInstruction = LLVMIsAShuffleVectorInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `store`.
    StoreInstruction = LLVMIsAStoreInst as Dynamic: [UserMarker, InstructionMarker, AlignedMarker, VolatileMarker];
    /// A `br`, with or without a condition.
    BranchInstruction = LLVMIsABranchInst as Dynamic: [UserMarker, InstructionMarker, TerminatorMarker];
    /// A `br` with no condition.
    UnconditionalBranchInstruction = LLVMIsAUncondBrInst as Dynamic: [UserMarker, InstructionMarker, TerminatorMarker];
    /// A `br` with a condition.
    ConditionalBranchInstruction = LLVMIsACondBrInst as Dynamic: [UserMarker, InstructionMarker, TerminatorMarker];
    /// An `indirectbr`.
    IndirectBranchInstruction = LLVMIsAIndirectBrInst as Dynamic: [UserMarker, InstructionMarker, TerminatorMarker];
    /// An `invoke`.
    InvokeInstruction = LLVMIsAInvokeInst as Dynamic: [UserMarker, InstructionMarker, CallBaseMarker, TerminatorMarker];
    /// A `ret`.
    ReturnInstruction = LLVMIsAReturnInst as Dynamic: [UserMarker, InstructionMarker, TerminatorMarker];
    /// A `switch`.
    SwitchInstruction = LLVMIsASwitchInst as Dynamic: [UserMarker, InstructionMarker, TerminatorMarker];
    /// An `unreachable`.
    UnreachableInstruction = LLVMIsAUnreachableInst as Dynamic: [UserMarker, InstructionMarker, TerminatorMarker];
    /// A `resume`.
    ResumeInstruction = LLVMIsAResumeInst as Dynamic: [UserMarker, InstructionMarker, TerminatorMarker];
    /// A `cleanupret`.
    CleanupReturnInstruction = LLVMIsACleanupReturnInst as Dynamic: [UserMarker, InstructionMarker, TerminatorMarker];
    /// A `catchret`.
    CatchReturnInstruction = LLVMIsACatchReturnInst as Dynamic: [UserMarker, InstructionMarker, TerminatorMarker];
    /// A `catchswitch`.
    CatchSwitchInstruction = LLVMIsACatchSwitchInst as Dynamic: [UserMarker, InstructionMarker, TerminatorMarker];
    /// A `callbr`.
    CallBranchInstruction = LLVMIsACallBrInst as Dynamic:
        [UserMarker, InstructionMarker, CallBaseMarker, TerminatorMarker];
    /// A `catchpad` or `cleanuppad`.
    FuncletPadInstruction = LLVMIsAFuncletPadInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `catchpad`.
    CatchPadInstruction = LLVMIsACatchPadInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `cleanuppad`.
    CleanupPadInstruction = LLVMIsACleanupPadInst as Dynamic: [UserMarker, InstructionMarker];
    /// An instruction with one operand.
    UnaryInstruction = LLVMIsAUnaryInstruction as Dynamic: [UserMarker, InstructionMarker];
    /// An `alloca`.
    StackAllocationInstruction = LLVMIsAAllocaInst as Dynamic: [UserMarker, InstructionMarker, AlignedMarker];
    /// A cast.
    CastInstruction = LLVMIsACastInst as Dynamic: [UserMarker, InstructionMarker];
    /// An `addrspacecast`.
    AddressSpaceCastInstruction = LLVMIsAAddrSpaceCastInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `bitcast`.
    BitCastInstruction = LLVMIsABitCastInst as Dynamic: [UserMarker, InstructionMarker];
    /// An `fpext`.
    FloatExtendInstruction = LLVMIsAFPExtInst as Dynamic: [UserMarker, InstructionMarker];
    /// An `fptosi`.
    FloatToSignedIntegerInstruction = LLVMIsAFPToSIInst as Dynamic: [UserMarker, InstructionMarker];
    /// An `fptoui`.
    FloatToUnsignedIntegerInstruction = LLVMIsAFPToUIInst as Dynamic: [UserMarker, InstructionMarker];
    /// An `fptrunc`.
    FloatTruncateInstruction = LLVMIsAFPTruncInst as Dynamic: [UserMarker, InstructionMarker];
    /// An `inttoptr`.
    IntegerToPointerInstruction = LLVMIsAIntToPtrInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `ptrtoint`.
    PointerToIntegerInstruction = LLVMIsAPtrToIntInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `sext`.
    SignExtendInstruction = LLVMIsASExtInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `sitofp`.
    SignedIntegerToFloatInstruction = LLVMIsASIToFPInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `trunc`.
    TruncateInstruction = LLVMIsATruncInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `uitofp`.
    UnsignedIntegerToFloatInstruction = LLVMIsAUIToFPInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `zext`.
    ZeroExtendInstruction = LLVMIsAZExtInst as Dynamic: [UserMarker, InstructionMarker];
    /// An `extractvalue`.
    ExtractValueInstruction = LLVMIsAExtractValueInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `load`.
    LoadInstruction = LLVMIsALoadInst as Dynamic: [UserMarker, InstructionMarker, AlignedMarker, VolatileMarker];
    /// A `va_arg`.
    VariableArgumentInstruction = LLVMIsAVAArgInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `freeze`.
    FreezeInstruction = LLVMIsAFreezeInst as Dynamic: [UserMarker, InstructionMarker];
    /// A `cmpxchg`.
    AtomicCompareExchangeInstruction = LLVMIsAAtomicCmpXchgInst as Dynamic:
        [UserMarker, InstructionMarker, AlignedMarker, VolatileMarker, AtomicMarker];
    /// An `atomicrmw`.
    AtomicReadModifyWriteInstruction = LLVMIsAAtomicRMWInst as Dynamic:
        [UserMarker, InstructionMarker, AlignedMarker, VolatileMarker, AtomicMarker];
    /// A `fence`.
    FenceInstruction = LLVMIsAFenceInst as Dynamic: [UserMarker, InstructionMarker, AtomicMarker];
}

/// Any instruction that calls something (LLVM's `CallBase`): a call, an invoke or a call branch.
#[derive(Clone, Copy, Debug)]
pub struct CallBase;
impl Sealed for CallBase {}
impl ValueMarker for CallBase {
    type LlvmType = Dynamic;
    unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool {
        // SAFETY: the caller promises a live value.
        unsafe {
            !raw::LLVMIsACallInst(raw).is_null()
                || !raw::LLVMIsAInvokeInst(raw).is_null()
                || !raw::LLVMIsACallBrInst(raw).is_null()
        }
    }
}
impl UserMarker for CallBase {}
impl InstructionMarker for CallBase {}
impl CallBaseMarker for CallBase {}

/// Any block terminator.
#[derive(Clone, Copy, Debug)]
pub struct Terminator;
impl Sealed for Terminator {}
impl ValueMarker for Terminator {
    type LlvmType = Dynamic;
    unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool {
        // SAFETY: the caller promises a live value.
        unsafe { !raw::LLVMIsATerminatorInst(raw).is_null() }
    }
}
impl UserMarker for Terminator {}
impl InstructionMarker for Terminator {}
impl TerminatorMarker for Terminator {}

impl Value<'_, '_, ConstantByte> {
    /// This constant, zero-extended, assuming it fits in 64 bits, as LLVM's C API does.
    #[must_use]
    pub fn zero_extended_value(self) -> u64 {
        // SAFETY: the marker guarantees a constant byte.
        unsafe { raw::LLVMConstByteGetZExtValue(self.raw) }
    }

    /// This constant, sign-extended, assuming it fits in 64 bits, as LLVM's C API does.
    #[must_use]
    pub fn sign_extended_value(self) -> i64 {
        // SAFETY: the marker guarantees a constant byte.
        unsafe { raw::LLVMConstByteGetSExtValue(self.raw) }
    }
}

/// A use of a value: one operand of one user.
#[derive(Clone, Copy)]
pub struct Use<'context, 'module> {
    raw: raw::LLVMUseRef,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl<'context, 'module> Use<'context, 'module> {
    /// Wraps a raw use.
    ///
    /// # Safety
    /// `raw` must be a live use that stays live for `'module`.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMUseRef) -> Self {
        Use { raw, marker: PhantomData }
    }

    /// The raw use.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(self) -> raw::LLVMUseRef {
        self.raw
    }

    /// The value that uses it.
    #[must_use]
    pub fn user(self) -> Value<'context, 'module, User> {
        // SAFETY: the use is live for `'module`, and so is its user.
        unsafe { Value::from_raw(raw::LLVMGetUser(self.raw)) }
    }

    /// The value being used.
    #[must_use]
    pub fn used_value(self) -> Value<'context, 'module> {
        // SAFETY: the use is live for `'module`, and so is what it uses.
        unsafe { Value::from_raw(raw::LLVMGetUsedValue(self.raw)) }
    }
}

/// The uses of a value, from [`Value::uses`].
pub struct Uses<'context, 'module> {
    next: raw::LLVMUseRef,
    module: raw::LLVMModuleRef,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl<'context, 'module> Iterator for Uses<'context, 'module> {
    type Item = Use<'context, 'module>;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.next.is_null() {
            let current = self.next;
            // SAFETY: `current` is live, since nothing can delete it while `'module` lasts.
            self.next = unsafe { raw::LLVMGetNextUse(current) };
            // Only uses in this iterator's module, when it is for a constant shared across modules. A constant user that
            // reaches no global can be anywhere, and has no module of its own to hold on to, so it isn't one of them.
            // SAFETY: the use's user is live.
            let in_module = self.module.is_null()
                || unsafe {
                    let user = raw::LLVMGetUser(current);
                    !shim::lyftar_value_is_context_constant(user)
                        && shim::lyftar_value_belongs_to_module(user, self.module)
                };
            if in_module {
                // SAFETY: the same use.
                return Some(unsafe { Use::from_raw(current) });
            }
        }
        None
    }
}

impl<'context, 'module, T> Value<'context, 'module, T> {
    /// Wraps a raw value.
    ///
    /// # Safety
    /// `raw` must be a live value of this context, of the kind `T` says, and stay live for `'module`.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMValueRef) -> Self {
        Value { raw, marker: PhantomData }
    }

    /// The raw value.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(self) -> raw::LLVMValueRef {
        self.raw
    }

    /// This value, with nothing known about it in the Rust code.
    #[must_use]
    pub fn to_dynamic(self) -> Value<'context, 'module> {
        Value { raw: self.raw, marker: PhantomData }
    }

    /// LLVM's text for this value.
    #[must_use]
    pub fn print_to_string(self) -> String {
        // SAFETY: the value is live for `'module`.
        from_owned_message(unsafe { raw::LLVMPrintValueToString(self.raw) })
    }

    /// What a panic calls this value: a global by name, since printing a function prints its whole body.
    #[must_use]
    pub(crate) fn describe(self) -> String {
        // SAFETY: the value is live for `'module`.
        if unsafe { !raw::LLVMIsAGlobalValue(self.raw).is_null() } {
            return format!("@{}", self.name());
        }
        self.print_to_string().trim().to_string()
    }

    /// This value's name, empty if it has none.
    #[must_use]
    pub fn name(self) -> String {
        let mut length = 0;
        // SAFETY: the value is live for `'module`, and the name is copied right away.
        let name = unsafe { raw::LLVMGetValueName2(self.raw, &raw mut length) };
        from_pointer_and_length(name, length)
    }

    /// Sets this value's name. A constant that isn't a global has no name to set.
    pub fn set_name(self, name: &str) {
        // SAFETY: the value is live; LLVM reads `name.len()` bytes; a void instruction can't be named, so it is skipped.
        unsafe {
            if raw::LLVMGetTypeKind(raw::LLVMTypeOf(self.raw)) != raw::LLVMTypeKind::LLVMVoidTypeKind {
                raw::LLVMSetValueName2(self.raw, name.as_ptr().cast(), name.len());
            }
        }
    }

    /// What kind of LLVM value this is.
    #[must_use]
    pub fn category(self) -> ValueCategory {
        // SAFETY: the value is live for `'module`.
        ValueCategory::from_raw(unsafe { raw::LLVMGetValueKind(self.raw) })
    }

    /// Whether this value is a constant.
    #[must_use]
    pub fn is_constant(self) -> bool {
        // SAFETY: the value is live for `'module`.
        unsafe { raw::LLVMIsConstant(self.raw) != 0 }
    }

    /// Whether this value is `undef`.
    #[must_use]
    pub fn is_undefined(self) -> bool {
        // SAFETY: the value is live for `'module`.
        unsafe { raw::LLVMIsUndef(self.raw) != 0 }
    }

    /// Whether this value is `poison`.
    #[must_use]
    pub fn is_poison(self) -> bool {
        // SAFETY: the value is live for `'module`.
        unsafe { raw::LLVMIsPoison(self.raw) != 0 }
    }

    /// Whether this value is a constant zero.
    #[must_use]
    pub fn is_null(self) -> bool {
        // SAFETY: the value is live for `'module`; LLVM answers false for anything that isn't a constant.
        unsafe { raw::LLVMIsNull(self.raw) != 0 }
    }

    /// The uses of this value.
    ///
    /// # Panics
    /// If this value belongs to the context, so its uses can be in any module: [`Value::uses_in`] says which (D3).
    #[must_use]
    #[track_caller]
    pub fn uses(self) -> Uses<'context, 'module> {
        // trace: D11
        // SAFETY: the value is live for `'module`; the shim only reads it.
        if unsafe { shim::lyftar_value_is_context_constant(self.raw) } {
            bad_call(format_args!(
                "uses: {} belongs to the context, so its uses can be in any module: use uses_in",
                self.describe()
            ));
        }
        // SAFETY: the value is tied to a module, as just checked, which is where all its uses are, live for `'module`.
        Uses { next: unsafe { raw::LLVMGetFirstUse(self.raw) }, module: std::ptr::null_mut(), marker: PhantomData }
    }

    /// The uses of this value in one module, for a constant that belongs to the context and can be used in several.
    #[must_use]
    pub fn uses_in<'borrow>(self, module: &'borrow Module<'context>) -> Uses<'context, 'borrow> {
        // SAFETY: the value is live; the iterator gives only the uses in this module, which last as long as this borrow.
        Uses { next: unsafe { raw::LLVMGetFirstUse(self.raw) }, module: module.raw, marker: PhantomData }
    }

    // trace: D11
    /// Replaces every use of this value with `replacement`.
    ///
    /// This takes only a shared borrow, so it refuses when LLVM would delete something in doing it, which a `Value` or
    /// `Metadata` might still point at: a constant that uses this value, or the metadata around it. For those,
    /// [`Module::replacement_to_make`](crate::Module::replacement_to_make) and [`Module::replace`](crate::Module::replace)
    /// take exclusive access, so nothing can be held across them (D34).
    ///
    /// # Panics
    /// If the two types differ, if this value belongs only to the context, whose uses can be in any module, if a constant
    /// other than a global uses it, or if metadata refers to it and LLVM can't move that over to `replacement` (D3).
    #[track_caller]
    pub fn replace_all_uses_with<U>(self, replacement: Value<'context, 'module, U>) {
        // SAFETY: both are live for `'module`.
        let (own_type, other_type) = unsafe { (raw::LLVMTypeOf(self.raw), raw::LLVMTypeOf(replacement.raw)) };
        if !checks::same_identity(own_type as usize, other_type as usize) {
            bad_call(format_args!(
                "replace_all_uses_with: {} and {} have different types",
                self.describe(),
                replacement.describe()
            ));
        }
        // SAFETY: both are live.
        if unsafe { !shim::lyftar_value_can_replace_uses(self.raw, replacement.raw) } {
            bad_call(format_args!(
                "replace_all_uses_with: can't replace the uses of {} with {}",
                self.describe(),
                replacement.describe()
            ));
        }
        // SAFETY: both are live, and the check above means this one keeps a use list.
        if unsafe { shim::lyftar_value_has_constant_user(self.raw) } {
            bad_call(format_args!(
                "replace_all_uses_with: a constant uses {}, and LLVM can delete that constant; \
                 Module::replacement_to_make and Module::replace do this with exclusive access",
                self.describe()
            ));
        }
        // SAFETY: both are live.
        if unsafe { !shim::lyftar_replacing_keeps_metadata(self.raw, replacement.raw) } {
            bad_call(format_args!(
                "replace_all_uses_with: metadata refers to {}, and LLVM can delete it for {}; \
                 Module::replacement_to_make and Module::replace do this with exclusive access",
                self.describe(),
                replacement.describe()
            ));
        }
        // SAFETY: just checked what LLVM asserts, and that it deletes nothing a `Value` or `Metadata` can point at.
        unsafe { raw::LLVMReplaceAllUsesWith(self.raw, replacement.raw) };
    }
}

impl<'context, 'module, T: ValueMarker> Value<'context, 'module, T> {
    /// This value's type.
    #[must_use]
    pub fn get_type(self) -> Type<'context, T::LlvmType> {
        // SAFETY: the value is live, and `T` says what its type is.
        unsafe { Type::from_raw(raw::LLVMTypeOf(self.raw)) }
    }

    /// This value as a `U`, if it is one (D4).
    #[must_use]
    pub fn as_kind<U: ValueMarker>(self) -> Option<Value<'context, 'module, U>> {
        // SAFETY: the value is live for `'module`.
        unsafe { U::accepts_value(self.raw) }.then_some(Value { raw: self.raw, marker: PhantomData })
    }

    // trace: D4
    /// This value as a `U`, checked here.
    ///
    /// # Panics
    /// If it isn't one (D3).
    #[must_use]
    #[track_caller]
    pub fn checked_cast<U: ValueMarker>(self) -> Value<'context, 'module, U> {
        match self.as_kind::<U>() {
            Some(value) => value,
            None => bad_call(format_args!("checked_cast: {} is not a {}", self.describe(), std::any::type_name::<U>())),
        }
    }
}

impl<'context, 'module, T: UserMarker> Value<'context, 'module, T> {
    /// How many operands this value has.
    #[must_use]
    pub fn operand_count(self) -> u32 {
        // SAFETY: `UserMarker` guarantees a user, which is what LLVM asks for. A count is never negative.
        unsafe { raw::LLVMGetNumOperands(self.raw) }.cast_unsigned()
    }

    #[track_caller]
    fn check_operand_index(self, operation: &str, index: u32) {
        let count = self.operand_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("{operation}: index {index} is out of bounds for {count} operands"));
        }
    }

    /// The operand at `index`. Operands that are blocks come back as values too.
    ///
    /// # Panics
    /// If there is no such operand (D3).
    #[must_use]
    #[track_caller]
    pub fn operand(self, index: u32) -> Value<'context, 'module> {
        self.check_operand_index("operand", index);
        // SAFETY: the index was just checked, and the user is live for `'module`.
        unsafe { Value::from_raw(raw::LLVMGetOperand(self.raw, index)) }
    }

    /// The use at operand `index`.
    ///
    /// # Panics
    /// If there is no such operand (D3).
    #[must_use]
    #[track_caller]
    pub fn operand_use(self, index: u32) -> Use<'context, 'module> {
        self.check_operand_index("operand_use", index);
        // SAFETY: the index was just checked, and the user is live for `'module`.
        unsafe { Use::from_raw(raw::LLVMGetOperandUse(self.raw, index)) }
    }

    /// Sets the operand at `index`.
    ///
    /// # Panics
    /// If there is no such operand, or the new one has another type, or it belongs to another module, or this is a constant
    /// other than a global, whose operands LLVM keeps fixed because it shares one copy of each constant (D3).
    #[track_caller]
    pub fn set_operand<U>(self, index: u32, operand: Value<'context, 'module, U>) {
        self.check_operand_index("set_operand", index);
        // SAFETY: the value is live; the shim only reads it.
        if !unsafe { shim::lyftar_value_operands_can_change(self.raw) } {
            bad_call(format_args!("set_operand: the operands of {} can't change", self.describe()));
        }
        // SAFETY: the index was just checked and both values are live.
        unsafe {
            let old = raw::LLVMGetOperand(self.raw, index);
            if !checks::same_identity(raw::LLVMTypeOf(old) as usize, raw::LLVMTypeOf(operand.raw) as usize) {
                bad_call(format_args!(
                    "set_operand: {} doesn't have the type of operand {index} of {}",
                    operand.describe(),
                    self.describe()
                ));
            }
            if !shim::lyftar_values_share_module(self.raw, operand.raw) {
                bad_call(format_args!(
                    "set_operand: {} belongs to another module than {}",
                    operand.describe(),
                    self.describe()
                ));
            }
            raw::LLVMSetOperand(self.raw, index, operand.raw);
        }
    }
}

impl<'context, 'module> Value<'context, 'module, BasicBlockAsValue> {
    /// This value as a block.
    #[must_use]
    pub fn to_basic_block(self) -> BasicBlock<'context, 'module> {
        // SAFETY: `BasicBlockAsValue` guarantees a block, which is live for `'module`.
        unsafe { BasicBlock::from_raw(raw::LLVMValueAsBasicBlock(self.raw)) }
    }
}

/// Metadata wrapped as a value, as an intrinsic's argument takes it.
#[derive(Clone, Copy, Debug)]
pub struct MetadataAsValue;
impl Sealed for MetadataAsValue {}
impl ValueMarker for MetadataAsValue {
    type LlvmType = Dynamic;
    unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool {
        // SAFETY: the caller promises a live value.
        unsafe { raw::LLVMGetValueKind(raw) == raw::LLVMValueKind::LLVMMetadataAsValueValueKind }
    }
}

/// A `phi`. It says what it holds, as `PhiNode<Integer<Bits<64>>>`, and [`Value::as_value`] gives its value.
#[derive(Clone, Copy, Debug)]
pub struct PhiNode<T = Dynamic>(PhantomData<fn() -> T>);
impl<T: TypeMarker> Sealed for PhiNode<T> {}
impl<T: TypeMarker> ValueMarker for PhiNode<T> {
    type LlvmType = T;
    unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool {
        // SAFETY: the caller promises a live value, and a live value has a live type.
        unsafe { !raw::LLVMIsAPHINode(raw).is_null() && T::accepts_type(raw::LLVMTypeOf(raw)) }
    }
}
impl<T: TypeMarker> UserMarker for PhiNode<T> {}
impl<T: TypeMarker> InstructionMarker for PhiNode<T> {}
