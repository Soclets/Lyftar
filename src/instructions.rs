//! Asking about an instruction and changing one, once it is built.

use std::ffi::c_uint;

use crate::attributes::{Attribute, AttributeLocation};
use crate::bad_call::bad_call;
use crate::block::Block;
use crate::checks;
use crate::enums::{
    AtomicOrdering, AtomicReadModifyWriteOperation, FloatPredicate, IntegerPredicate, Opcode, TailCallKind,
};
use crate::metadata::{Metadata, MetadataKind};
use crate::raw;
use crate::shim;
use crate::text::from_pointer_and_length;
use crate::types::{Integer, Type, TypeMarker, length_as_u32};
use crate::values::{
    AlignedMarker, AtomicCompareExchangeInstruction, AtomicMarker, AtomicReadModifyWriteInstruction, CallBaseMarker,
    CallBranchInstruction, CatchPadInstruction, CatchSwitchInstruction, ConditionalBranchInstruction, ConstantInteger,
    ExtractValueInstruction, FloatCompareInstruction, FuncletPadInstruction, Function, GetElementPointerInstruction,
    IndirectBranchInstruction, InsertValueInstruction, Instruction, InstructionMarker, IntegerCompareInstruction,
    IntrinsicInstruction, InvokeInstruction, LandingPadInstruction, LoadInstruction, MemoryIntrinsic, PhiNode,
    ShuffleVectorInstruction, StackAllocationInstruction, StoreInstruction, SwitchInstruction, TerminatorMarker, Value,
    VolatileMarker,
};

impl<'context, 'module, T: InstructionMarker> Value<'context, 'module, T> {
    /// The block this instruction is in.
    #[must_use]
    pub fn parent_block(self) -> Block<'context, 'module> {
        // SAFETY: Lyftar only hands out instructions that are in a block.
        unsafe { Block::from_raw(raw::LLVMGetInstructionParent(self.raw)) }
    }

    /// The next instruction in the block.
    #[must_use]
    pub fn next_instruction(self) -> Option<Value<'context, 'module, Instruction>> {
        // SAFETY: the instruction is live for `'module`.
        let next = unsafe { raw::LLVMGetNextInstruction(self.raw) };
        // SAFETY: LLVM said it is an instruction, live for `'module`.
        (!next.is_null()).then(|| unsafe { Value::from_raw(next) })
    }

    /// The previous instruction in the block.
    #[must_use]
    pub fn previous_instruction(self) -> Option<Value<'context, 'module, Instruction>> {
        // SAFETY: the instruction is live for `'module`.
        let previous = unsafe { raw::LLVMGetPreviousInstruction(self.raw) };
        // SAFETY: LLVM said it is an instruction, live for `'module`.
        (!previous.is_null()).then(|| unsafe { Value::from_raw(previous) })
    }

    /// This instruction's operation.
    #[must_use]
    pub fn opcode(self) -> Opcode {
        // SAFETY: the marker guarantees an instruction.
        Opcode::from_raw(unsafe { raw::LLVMGetInstructionOpcode(self.raw) })
    }

    /// Whether this instruction has metadata, a debug location included.
    #[must_use]
    pub fn has_metadata(self) -> bool {
        // SAFETY: the marker guarantees an instruction, which is what this call wants.
        unsafe { raw::LLVMHasMetadata(self.raw) != 0 }
    }

    /// This instruction's metadata of a kind.
    #[must_use]
    pub fn metadata(self, kind: MetadataKind) -> Option<Metadata<'context, 'module>> {
        // SAFETY: the marker guarantees an instruction; the value LLVM gives back is metadata wrapped as a value.
        unsafe {
            let wrapped = raw::LLVMGetMetadata(self.raw, kind.0);
            (!wrapped.is_null()).then(|| Metadata::from_raw(raw::LLVMValueAsMetadata(wrapped)))
        }
    }

    /// Sets this instruction's metadata of a kind.
    pub fn set_metadata(self, kind: MetadataKind, node: Metadata<'context, 'module>) {
        // SAFETY: the marker guarantees an instruction, the metadata is live for `'module`, and LLVM wants it wrapped.
        unsafe {
            let context = raw::LLVMGetValueContext(self.raw);
            raw::LLVMSetMetadata(self.raw, kind.0, raw::LLVMMetadataAsValue(context, node.raw));
        }
    }

    /// Removes this instruction's metadata of a kind.
    pub fn clear_metadata(self, kind: MetadataKind) {
        // SAFETY: the marker guarantees an instruction, and a null node removes the metadata.
        unsafe { raw::LLVMSetMetadata(self.raw, kind.0, std::ptr::null_mut()) };
    }

    /// All of this instruction's metadata except its debug location.
    #[must_use]
    pub fn all_metadata_other_than_debug_location(self) -> Vec<(MetadataKind, Metadata<'context, 'module>)> {
        let mut count = 0;
        // SAFETY: the marker guarantees an instruction, and the entries are copied and freed right after.
        unsafe {
            let entries = raw::LLVMInstructionGetAllMetadataOtherThanDebugLoc(self.raw, &raw mut count);
            let found = crate::metadata::copy_entries(entries, count);
            found
        }
    }

    /// This instruction's debug location.
    #[must_use]
    pub fn debug_location(self) -> Option<Metadata<'context, 'module>> {
        // SAFETY: the marker guarantees an instruction.
        let location = unsafe { raw::LLVMInstructionGetDebugLoc(self.raw) };
        // SAFETY: LLVM gave back live metadata.
        (!location.is_null()).then(|| unsafe { Metadata::from_raw(location) })
    }

    /// Sets this instruction's debug location.
    pub fn set_debug_location(self, location: Metadata<'context, 'module>) {
        // SAFETY: the marker guarantees an instruction, and the metadata is live for `'module`.
        unsafe { raw::LLVMInstructionSetDebugLoc(self.raw, location.raw) };
    }

    /// The directory of this instruction's debug location.
    #[must_use]
    pub fn debug_location_directory(self) -> String {
        let mut length = 0;
        // SAFETY: the marker guarantees an instruction, one of the values LLVM takes here; copied right away.
        let text = unsafe { raw::LLVMGetDebugLocDirectory(self.raw, &raw mut length) };
        from_pointer_and_length(text, length as usize)
    }

    /// The file name of this instruction's debug location.
    #[must_use]
    pub fn debug_location_filename(self) -> String {
        let mut length = 0;
        // SAFETY: the marker guarantees an instruction, one of the values LLVM takes here; copied right away.
        let text = unsafe { raw::LLVMGetDebugLocFilename(self.raw, &raw mut length) };
        from_pointer_and_length(text, length as usize)
    }

    /// The line of this instruction's debug location.
    #[must_use]
    pub fn debug_location_line(self) -> u32 {
        // SAFETY: the marker guarantees an instruction, one of the values LLVM takes here.
        unsafe { raw::LLVMGetDebugLocLine(self.raw) }
    }

    /// The column of this instruction's debug location.
    #[must_use]
    pub fn debug_location_column(self) -> u32 {
        // SAFETY: the marker guarantees an instruction.
        unsafe { raw::LLVMGetDebugLocColumn(self.raw) }
    }

    /// Whether this instruction can have fast-math flags.
    #[must_use]
    pub fn can_use_fast_math_flags(self) -> bool {
        // SAFETY: the marker guarantees an instruction.
        unsafe { raw::LLVMCanValueUseFastMathFlags(self.raw) != 0 }
    }

    #[track_caller]
    fn require_opcode(self, operation: &str, allowed: &[Opcode], what: &str) {
        if !allowed.contains(&self.opcode()) {
            bad_call(format_args!("{operation}: {} is not {what}", self.describe()));
        }
    }

    /// Whether this add, subtract, multiply, shift left or negate wraps no unsigned value.
    ///
    /// # Panics
    /// If it isn't one of those (D3).
    #[must_use]
    #[track_caller]
    pub fn has_no_unsigned_wrap(self) -> bool {
        self.require_opcode("has_no_unsigned_wrap", &WRAP_OPCODES, "an add, subtract, multiply or shift left");
        // SAFETY: just checked what LLVM asks for.
        unsafe { raw::LLVMGetNUW(self.raw) != 0 }
    }

    /// Sets whether this add, subtract, multiply or shift left wraps no unsigned value.
    ///
    /// # Panics
    /// If it isn't one of those (D3).
    #[track_caller]
    pub fn set_no_unsigned_wrap(self, value: bool) {
        self.require_opcode("set_no_unsigned_wrap", &WRAP_OPCODES, "an add, subtract, multiply or shift left");
        // SAFETY: just checked what LLVM asks for.
        unsafe { raw::LLVMSetNUW(self.raw, i32::from(value)) };
    }

    /// Whether this add, subtract, multiply or shift left wraps no signed value.
    ///
    /// # Panics
    /// If it isn't one of those (D3).
    #[must_use]
    #[track_caller]
    pub fn has_no_signed_wrap(self) -> bool {
        self.require_opcode("has_no_signed_wrap", &WRAP_OPCODES, "an add, subtract, multiply or shift left");
        // SAFETY: just checked what LLVM asks for.
        unsafe { raw::LLVMGetNSW(self.raw) != 0 }
    }

    /// Sets whether this add, subtract, multiply or shift left wraps no signed value.
    ///
    /// # Panics
    /// If it isn't one of those (D3).
    #[track_caller]
    pub fn set_no_signed_wrap(self, value: bool) {
        self.require_opcode("set_no_signed_wrap", &WRAP_OPCODES, "an add, subtract, multiply or shift left");
        // SAFETY: just checked what LLVM asks for.
        unsafe { raw::LLVMSetNSW(self.raw, i32::from(value)) };
    }

    /// Whether this divide or right shift is exact.
    ///
    /// # Panics
    /// If it isn't one of those (D3).
    #[must_use]
    #[track_caller]
    pub fn is_exact(self) -> bool {
        self.require_opcode("is_exact", &EXACT_OPCODES, "a divide or a right shift");
        // SAFETY: just checked what LLVM asks for.
        unsafe { raw::LLVMGetExact(self.raw) != 0 }
    }

    /// Sets whether this divide or right shift is exact.
    ///
    /// # Panics
    /// If it isn't one of those (D3).
    #[track_caller]
    pub fn set_exact(self, value: bool) {
        self.require_opcode("set_exact", &EXACT_OPCODES, "a divide or a right shift");
        // SAFETY: just checked what LLVM asks for.
        unsafe { raw::LLVMSetExact(self.raw, i32::from(value)) };
    }

    /// Whether this zero extend or unsigned-to-float cast promises a non-negative operand.
    ///
    /// # Panics
    /// If it isn't one of those (D3).
    #[must_use]
    #[track_caller]
    pub fn has_non_negative(self) -> bool {
        self.require_opcode("has_non_negative", &NON_NEGATIVE_OPCODES, "a zero extend or unsigned-to-float cast");
        // SAFETY: just checked what LLVM asks for.
        unsafe { raw::LLVMGetNNeg(self.raw) != 0 }
    }

    /// Sets whether this zero extend or unsigned-to-float cast promises a non-negative operand.
    ///
    /// # Panics
    /// If it isn't one of those (D3).
    #[track_caller]
    pub fn set_non_negative(self, value: bool) {
        self.require_opcode("set_non_negative", &NON_NEGATIVE_OPCODES, "a zero extend or unsigned-to-float cast");
        // SAFETY: just checked what LLVM asks for.
        unsafe { raw::LLVMSetNNeg(self.raw, i32::from(value)) };
    }

    /// Whether this `or` promises its operands share no set bit.
    ///
    /// # Panics
    /// If it isn't an `or` (D3).
    #[must_use]
    #[track_caller]
    pub fn is_disjoint(self) -> bool {
        self.require_opcode("is_disjoint", &[Opcode::Or], "an `or`");
        // SAFETY: just checked what LLVM asks for.
        unsafe { raw::LLVMGetIsDisjoint(self.raw) != 0 }
    }

    /// Sets whether this `or` promises its operands share no set bit.
    ///
    /// # Panics
    /// If it isn't an `or` (D3).
    #[track_caller]
    pub fn set_disjoint(self, value: bool) {
        self.require_opcode("set_disjoint", &[Opcode::Or], "an `or`");
        // SAFETY: just checked what LLVM asks for.
        unsafe { raw::LLVMSetIsDisjoint(self.raw, i32::from(value)) };
    }

    /// This instruction's fast-math flags.
    ///
    /// # Panics
    /// If it can't have any (D3).
    #[must_use]
    #[track_caller]
    pub fn fast_math_flags(self) -> FastMathFlags {
        self.require_fast_math("fast_math_flags");
        // SAFETY: just checked what LLVM asks for.
        FastMathFlags(unsafe { raw::LLVMGetFastMathFlags(self.raw) })
    }

    /// Sets this instruction's fast-math flags.
    ///
    /// # Panics
    /// If it can't have any (D3).
    #[track_caller]
    pub fn set_fast_math_flags(self, flags: FastMathFlags) {
        self.require_fast_math("set_fast_math_flags");
        // SAFETY: just checked what LLVM asks for.
        unsafe { raw::LLVMSetFastMathFlags(self.raw, flags.0) };
    }

    #[track_caller]
    fn require_fast_math(self, operation: &str) {
        if !self.can_use_fast_math_flags() {
            bad_call(format_args!("{operation}: {} can't have fast-math flags", self.describe()));
        }
    }
}

const WRAP_OPCODES: [Opcode; 4] =
    [Opcode::IntegerAdd, Opcode::IntegerSubtract, Opcode::IntegerMultiply, Opcode::ShiftLeft];
const EXACT_OPCODES: [Opcode; 4] =
    [Opcode::UnsignedDivide, Opcode::SignedDivide, Opcode::LogicalShiftRight, Opcode::ArithmeticShiftRight];
const NON_NEGATIVE_OPCODES: [Opcode; 2] = [Opcode::ZeroExtend, Opcode::UnsignedIntegerToFloat];

/// Fast-math flags, which can be combined with `|`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FastMathFlags(pub(crate) raw::LLVMFastMathFlags);

impl FastMathFlags {
    /// No flags.
    pub const NONE: Self = FastMathFlags(0);
    /// `reassoc`.
    pub const ALLOW_REASSOCIATION: Self = FastMathFlags(1 << 0);
    /// `nnan`.
    pub const NO_NOT_A_NUMBERS: Self = FastMathFlags(1 << 1);
    /// `ninf`.
    pub const NO_INFINITIES: Self = FastMathFlags(1 << 2);
    /// `nsz`.
    pub const NO_SIGNED_ZEROS: Self = FastMathFlags(1 << 3);
    /// `arcp`.
    pub const ALLOW_RECIPROCAL: Self = FastMathFlags(1 << 4);
    /// `contract`.
    pub const ALLOW_CONTRACTION: Self = FastMathFlags(1 << 5);
    /// `afn`.
    pub const APPROXIMATE_FUNCTIONS: Self = FastMathFlags(1 << 6);
    /// All of them: `fast`.
    pub const ALL: Self = FastMathFlags(0x7f);
}

impl std::ops::BitOr for FastMathFlags {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        FastMathFlags(self.0 | other.0)
    }
}

/// A `getelementptr`'s no-wrap flags, which can be combined with `|`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GetElementPointerFlags(pub(crate) raw::LLVMGEPNoWrapFlags);

impl GetElementPointerFlags {
    /// No flags.
    pub const NONE: Self = GetElementPointerFlags(0);
    /// `inbounds`.
    pub const IN_BOUNDS: Self = GetElementPointerFlags(1 << 0);
    /// `nusw`.
    pub const NO_UNSIGNED_SIGNED_WRAP: Self = GetElementPointerFlags(1 << 1);
    /// `nuw`.
    pub const NO_UNSIGNED_WRAP: Self = GetElementPointerFlags(1 << 2);
}

impl std::ops::BitOr for GetElementPointerFlags {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        GetElementPointerFlags(self.0 | other.0)
    }
}

impl<T: AlignedMarker> Value<'_, '_, T> {
    /// This value's alignment.
    #[must_use]
    pub fn alignment_in_bytes(self) -> u32 {
        // SAFETY: `AlignedMarker` guarantees one of the classes LLVM takes here.
        unsafe { raw::LLVMGetAlignment(self.raw) }
    }

    /// Sets this value's alignment.
    ///
    /// # Panics
    /// If it isn't a power of two (D3).
    #[track_caller]
    pub fn set_alignment_in_bytes(self, alignment_in_bytes: u32) {
        if !checks::is_power_of_two(alignment_in_bytes) {
            bad_call(format_args!("set_alignment_in_bytes: {alignment_in_bytes} is not a power of two"));
        }
        // SAFETY: `AlignedMarker` guarantees one of the classes LLVM takes here, and the alignment was just checked.
        unsafe { raw::LLVMSetAlignment(self.raw, alignment_in_bytes) };
    }
}

impl<T: VolatileMarker> Value<'_, '_, T> {
    /// Whether this memory access is volatile.
    #[must_use]
    pub fn is_volatile(self) -> bool {
        // SAFETY: `VolatileMarker` guarantees one of the classes LLVM takes here.
        unsafe { raw::LLVMGetVolatile(self.raw) != 0 }
    }

    /// Sets whether this memory access is volatile.
    pub fn set_volatile(self, value: bool) {
        // SAFETY: `VolatileMarker` guarantees one of the classes LLVM takes here.
        unsafe { raw::LLVMSetVolatile(self.raw, i32::from(value)) };
    }
}

impl<T: AtomicMarker> Value<'_, '_, T> {
    /// Sets whether this operation only synchronizes with its own thread.
    pub fn set_single_thread(self, value: bool) {
        // SAFETY: `AtomicMarker` guarantees an atomic instruction.
        unsafe { raw::LLVMSetAtomicSingleThread(self.raw, i32::from(value)) };
    }

    /// This operation's synchronization scope.
    #[must_use]
    pub fn synchronization_scope(self) -> SynchronizationScope {
        // SAFETY: `AtomicMarker` guarantees an atomic instruction.
        SynchronizationScope(unsafe { raw::LLVMGetAtomicSyncScopeID(self.raw) })
    }

    /// Sets this operation's synchronization scope.
    pub fn set_synchronization_scope(self, scope: SynchronizationScope) {
        // SAFETY: `AtomicMarker` guarantees an atomic instruction.
        unsafe { raw::LLVMSetAtomicSyncScopeID(self.raw, scope.0) };
    }
}

/// A synchronization scope in a context, from `Context::synchronization_scope`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SynchronizationScope(pub(crate) c_uint);

/// Which memory orderings a load takes (LLVM's verifier: neither release nor acquire-release).
fn load_ordering_is_valid(ordering: AtomicOrdering) -> bool {
    !matches!(ordering, AtomicOrdering::Release | AtomicOrdering::AcquireRelease)
}

/// Which memory orderings a store takes (LLVM's verifier: neither acquire nor acquire-release).
fn store_ordering_is_valid(ordering: AtomicOrdering) -> bool {
    !matches!(ordering, AtomicOrdering::Acquire | AtomicOrdering::AcquireRelease)
}

impl Value<'_, '_, LoadInstruction> {
    /// This load's memory ordering.
    #[must_use]
    pub fn ordering(self) -> AtomicOrdering {
        // SAFETY: the marker guarantees a load.
        AtomicOrdering::from_raw(unsafe { raw::LLVMGetOrdering(self.raw) })
    }

    /// Sets this load's memory ordering.
    ///
    /// # Panics
    /// If a load can't have that ordering (D3).
    #[track_caller]
    pub fn set_ordering(self, ordering: AtomicOrdering) {
        if !load_ordering_is_valid(ordering) {
            bad_call(format_args!("set_ordering: a load can't be {ordering:?}"));
        }
        // SAFETY: the marker guarantees a load, and the ordering was just checked.
        unsafe { raw::LLVMSetOrdering(self.raw, ordering.to_raw()) };
    }
}

impl Value<'_, '_, StoreInstruction> {
    /// This store's memory ordering.
    #[must_use]
    pub fn ordering(self) -> AtomicOrdering {
        // SAFETY: the marker guarantees a store.
        AtomicOrdering::from_raw(unsafe { raw::LLVMGetOrdering(self.raw) })
    }

    /// Sets this store's memory ordering.
    ///
    /// # Panics
    /// If a store can't have that ordering (D3).
    #[track_caller]
    pub fn set_ordering(self, ordering: AtomicOrdering) {
        if !store_ordering_is_valid(ordering) {
            bad_call(format_args!("set_ordering: a store can't be {ordering:?}"));
        }
        // SAFETY: the marker guarantees a store, and the ordering was just checked.
        unsafe { raw::LLVMSetOrdering(self.raw, ordering.to_raw()) };
    }
}

impl<'context, 'module> Value<'context, 'module, StoreInstruction> {
    /// The pointer stored through.
    #[must_use]
    pub fn pointer_operand(self) -> Value<'context, 'module, crate::types::Pointer> {
        // SAFETY: a store's second operand is its pointer.
        unsafe { Value::from_raw(raw::LLVMGetOperand(self.raw, 1)) }
    }

    /// The value stored.
    #[must_use]
    pub fn value_operand(self) -> Value<'context, 'module> {
        // SAFETY: a store's first operand is its value.
        unsafe { Value::from_raw(raw::LLVMGetOperand(self.raw, 0)) }
    }
}

impl<'context, 'module> Value<'context, 'module, LoadInstruction> {
    /// The pointer loaded through.
    #[must_use]
    pub fn pointer_operand(self) -> Value<'context, 'module, crate::types::Pointer> {
        // SAFETY: a load's one operand is its pointer.
        unsafe { Value::from_raw(raw::LLVMGetOperand(self.raw, 0)) }
    }
}

impl Value<'_, '_, AtomicReadModifyWriteInstruction> {
    /// This operation's memory ordering.
    #[must_use]
    pub fn ordering(self) -> AtomicOrdering {
        // SAFETY: the marker guarantees an `atomicrmw`.
        AtomicOrdering::from_raw(unsafe { raw::LLVMGetOrdering(self.raw) })
    }

    /// Sets this operation's memory ordering.
    ///
    /// # Panics
    /// If it isn't an atomic ordering stronger than unordered (D3).
    #[track_caller]
    pub fn set_ordering(self, ordering: AtomicOrdering) {
        if matches!(ordering, AtomicOrdering::NotAtomic | AtomicOrdering::Unordered) {
            bad_call(format_args!("set_ordering: an atomicrmw can't be {ordering:?}"));
        }
        // SAFETY: the marker guarantees an `atomicrmw`, and the ordering was just checked.
        unsafe { raw::LLVMSetOrdering(self.raw, ordering.to_raw()) };
    }

    /// What this instruction does.
    #[must_use]
    pub fn operation(self) -> AtomicReadModifyWriteOperation {
        // SAFETY: the marker guarantees an `atomicrmw`.
        AtomicReadModifyWriteOperation::from_raw(unsafe { raw::LLVMGetAtomicRMWBinOp(self.raw) })
    }

    /// Sets what this instruction does.
    pub fn set_operation(self, operation: AtomicReadModifyWriteOperation) {
        // SAFETY: the marker guarantees an `atomicrmw`.
        unsafe { raw::LLVMSetAtomicRMWBinOp(self.raw, operation.to_raw()) };
    }
}

impl Value<'_, '_, AtomicCompareExchangeInstruction> {
    /// Whether this exchange can fail spuriously.
    #[must_use]
    pub fn is_weak(self) -> bool {
        // SAFETY: the marker guarantees a `cmpxchg`.
        unsafe { raw::LLVMGetWeak(self.raw) != 0 }
    }

    /// Sets whether this exchange can fail spuriously.
    pub fn set_weak(self, value: bool) {
        // SAFETY: the marker guarantees a `cmpxchg`.
        unsafe { raw::LLVMSetWeak(self.raw, i32::from(value)) };
    }

    /// The memory ordering when the exchange succeeds.
    #[must_use]
    pub fn success_ordering(self) -> AtomicOrdering {
        // SAFETY: the marker guarantees a `cmpxchg`.
        AtomicOrdering::from_raw(unsafe { raw::LLVMGetCmpXchgSuccessOrdering(self.raw) })
    }

    /// Sets the memory ordering when the exchange succeeds.
    ///
    /// # Panics
    /// If it isn't an atomic ordering stronger than unordered (D3).
    #[track_caller]
    pub fn set_success_ordering(self, ordering: AtomicOrdering) {
        if matches!(ordering, AtomicOrdering::NotAtomic | AtomicOrdering::Unordered) {
            bad_call(format_args!("set_success_ordering: a cmpxchg can't be {ordering:?}"));
        }
        // SAFETY: the marker guarantees a `cmpxchg`, and the ordering was just checked.
        unsafe { raw::LLVMSetCmpXchgSuccessOrdering(self.raw, ordering.to_raw()) };
    }

    /// The memory ordering when the exchange fails.
    #[must_use]
    pub fn failure_ordering(self) -> AtomicOrdering {
        // SAFETY: the marker guarantees a `cmpxchg`.
        AtomicOrdering::from_raw(unsafe { raw::LLVMGetCmpXchgFailureOrdering(self.raw) })
    }

    /// Sets the memory ordering when the exchange fails.
    ///
    /// # Panics
    /// If a failed exchange can't have that ordering (D3).
    #[track_caller]
    pub fn set_failure_ordering(self, ordering: AtomicOrdering) {
        if matches!(
            ordering,
            AtomicOrdering::NotAtomic
                | AtomicOrdering::Unordered
                | AtomicOrdering::Release
                | AtomicOrdering::AcquireRelease
        ) {
            bad_call(format_args!("set_failure_ordering: a failed cmpxchg can't be {ordering:?}"));
        }
        // SAFETY: the marker guarantees a `cmpxchg`, and the ordering was just checked.
        unsafe { raw::LLVMSetCmpXchgFailureOrdering(self.raw, ordering.to_raw()) };
    }
}

impl Value<'_, '_, IntegerCompareInstruction> {
    /// This comparison's predicate.
    #[must_use]
    pub fn predicate(self) -> IntegerPredicate {
        // SAFETY: the marker guarantees an `icmp`.
        IntegerPredicate::from_raw(unsafe { raw::LLVMGetICmpPredicate(self.raw) })
    }

    /// Whether this comparison promises both operands have the same sign.
    #[must_use]
    pub fn has_same_sign(self) -> bool {
        // SAFETY: the marker guarantees an `icmp`.
        unsafe { raw::LLVMGetICmpSameSign(self.raw) != 0 }
    }

    /// Sets whether this comparison promises both operands have the same sign.
    pub fn set_same_sign(self, value: bool) {
        // SAFETY: the marker guarantees an `icmp`.
        unsafe { raw::LLVMSetICmpSameSign(self.raw, i32::from(value)) };
    }
}

impl Value<'_, '_, FloatCompareInstruction> {
    /// This comparison's predicate.
    #[must_use]
    pub fn predicate(self) -> FloatPredicate {
        // SAFETY: the marker guarantees an `fcmp`.
        FloatPredicate::from_raw(unsafe { raw::LLVMGetFCmpPredicate(self.raw) })
    }
}

impl Value<'_, '_, crate::values::FenceInstruction> {
    /// This fence's memory ordering.
    #[must_use]
    pub fn ordering(self) -> AtomicOrdering {
        // SAFETY: the marker guarantees a `fence`.
        AtomicOrdering::from_raw(unsafe { raw::LLVMGetOrdering(self.raw) })
    }

    /// Sets this fence's memory ordering.
    ///
    /// # Panics
    /// If a fence can't have that ordering (D3).
    #[track_caller]
    pub fn set_ordering(self, ordering: AtomicOrdering) {
        if matches!(ordering, AtomicOrdering::NotAtomic | AtomicOrdering::Unordered | AtomicOrdering::Monotonic) {
            bad_call(format_args!("set_ordering: a fence can't be {ordering:?}"));
        }
        // SAFETY: the marker guarantees a `fence`, and the ordering was just checked.
        unsafe { raw::LLVMSetOrdering(self.raw, ordering.to_raw()) };
    }
}

impl<'context> Value<'context, '_, StackAllocationInstruction> {
    /// The type this allocates.
    #[must_use]
    pub fn allocated_type(self) -> Type<'context> {
        // SAFETY: the marker guarantees an `alloca`.
        unsafe { Type::from_raw(raw::LLVMGetAllocatedType(self.raw)) }
    }

    /// The size this allocates, when the data layout and a constant array size make it known here.
    #[must_use]
    pub fn allocation_size_in_bytes(self) -> Option<u64> {
        let mut size_in_bytes = 0;
        // SAFETY: the marker guarantees an `alloca`, which is in a module, and the shim writes one number.
        unsafe { shim::lyftar_allocation_size_in_bytes(self.raw, &raw mut size_in_bytes) }.then_some(size_in_bytes)
    }
}

impl<'context> Value<'context, '_, GetElementPointerInstruction> {
    /// The type this `getelementptr` indexes.
    #[must_use]
    pub fn source_element_type(self) -> Type<'context> {
        // SAFETY: the marker guarantees a `getelementptr`.
        unsafe { Type::from_raw(raw::LLVMGetGEPSourceElementType(self.raw)) }
    }

    /// Whether this `getelementptr` is `inbounds`.
    #[must_use]
    pub fn is_in_bounds(self) -> bool {
        // SAFETY: the marker guarantees a `getelementptr`.
        unsafe { raw::LLVMIsInBounds(self.raw) != 0 }
    }

    /// Sets whether this `getelementptr` is `inbounds`.
    pub fn set_in_bounds(self, value: bool) {
        // SAFETY: the marker guarantees a `getelementptr`.
        unsafe { raw::LLVMSetIsInBounds(self.raw, i32::from(value)) };
    }

    /// This `getelementptr`'s no-wrap flags.
    #[must_use]
    pub fn no_wrap_flags(self) -> GetElementPointerFlags {
        // SAFETY: the marker guarantees a `getelementptr`.
        GetElementPointerFlags(unsafe { raw::LLVMGEPGetNoWrapFlags(self.raw) })
    }

    /// Sets this `getelementptr`'s no-wrap flags.
    pub fn set_no_wrap_flags(self, flags: GetElementPointerFlags) {
        // SAFETY: the marker guarantees a `getelementptr`.
        unsafe { raw::LLVMGEPSetNoWrapFlags(self.raw, flags.0) };
    }
}

impl<'context, 'module> Value<'context, 'module, GetElementPointerInstruction> {
    /// The pointer this `getelementptr` starts from.
    #[must_use]
    pub fn pointer_operand(self) -> Value<'context, 'module> {
        // SAFETY: a `getelementptr`'s first operand is its pointer.
        unsafe { Value::from_raw(raw::LLVMGetOperand(self.raw, 0)) }
    }
}

impl<'context, 'module, T: TypeMarker> Value<'context, 'module, PhiNode<T>> {
    /// The value this phi gives.
    #[must_use]
    pub fn as_value(self) -> Value<'context, 'module, T> {
        // SAFETY: `PhiNode<T>` says the phi holds a `T`.
        unsafe { Value::from_raw(self.raw) }
    }

    /// How many incoming values this phi has.
    #[must_use]
    pub fn incoming_count(self) -> u32 {
        // SAFETY: the marker guarantees a `phi`.
        unsafe { raw::LLVMCountIncoming(self.raw) }
    }

    /// The incoming value at `index`.
    ///
    /// # Panics
    /// If there is no such incoming value (D3).
    #[must_use]
    #[track_caller]
    pub fn incoming_value(self, index: u32) -> Value<'context, 'module, T> {
        self.check_incoming("incoming_value", index);
        // SAFETY: the index was just checked, and each incoming value of a `phi` has its type.
        unsafe { Value::from_raw(raw::LLVMGetIncomingValue(self.raw, index)) }
    }

    /// The incoming block at `index`.
    ///
    /// # Panics
    /// If there is no such incoming value (D3).
    #[must_use]
    #[track_caller]
    pub fn incoming_block(self, index: u32) -> Block<'context, 'module> {
        self.check_incoming("incoming_block", index);
        // SAFETY: the index was just checked.
        unsafe { Block::from_raw(raw::LLVMGetIncomingBlock(self.raw, index)) }
    }

    #[track_caller]
    fn check_incoming(self, operation: &str, index: u32) {
        let count = self.incoming_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("{operation}: index {index} is out of bounds for {count} incoming values"));
        }
    }

    /// Adds incoming values, each with the block it comes from.
    ///
    /// # Panics
    /// If a value's type isn't this phi's, or a value or block is from another function or module (D3).
    #[track_caller]
    pub fn add_incoming(self, incoming: &[(Value<'context, 'module, T>, Block<'context, 'module>)]) {
        for (value, block) in incoming {
            // SAFETY: all three are live for `'module`.
            unsafe {
                if !T::EXACT
                    && !checks::same_identity(raw::LLVMTypeOf(value.raw) as usize, raw::LLVMTypeOf(self.raw) as usize)
                {
                    bad_call(format_args!(
                        "add_incoming: {} doesn't have the type of {}",
                        value.describe(),
                        self.describe()
                    ));
                }
                if !shim::lyftar_values_share_module(self.raw, value.raw) {
                    bad_call(format_args!("add_incoming: {} belongs to another module", value.describe()));
                }
                self.check_same_function("add_incoming", *block);
                let mut one_value = value.raw;
                let mut one_block = block.raw;
                raw::LLVMAddIncoming(self.raw, &raw mut one_value, &raw mut one_block, 1);
            }
        }
    }
}

impl<'context, 'module> Value<'context, 'module, SwitchInstruction> {
    /// The block a value matching no case goes to.
    #[must_use]
    pub fn default_destination(self) -> Block<'context, 'module> {
        // SAFETY: the marker guarantees a `switch`.
        unsafe { Block::from_raw(raw::LLVMGetSwitchDefaultDest(self.raw)) }
    }

    /// Adds a case.
    ///
    /// # Panics
    /// If the value's type differs from the condition's, or it isn't a constant integer, or the block is in another
    /// function (D3).
    #[track_caller]
    pub fn add_case<T>(self, on_value: Value<'context, 'module, T>, destination: Block<'context, 'module>) {
        // SAFETY: all are live for `'module`, and the condition is a `switch`'s first operand.
        unsafe {
            if raw::LLVMIsAConstantInt(on_value.raw).is_null()
                || !checks::same_identity(
                    raw::LLVMTypeOf(raw::LLVMGetOperand(self.raw, 0)) as usize,
                    raw::LLVMTypeOf(on_value.raw) as usize,
                )
            {
                bad_call(format_args!("add_case: {} is not a constant of the switch's type", on_value.describe()));
            }
            if raw::LLVMGetBasicBlockParent(destination.raw)
                != raw::LLVMGetBasicBlockParent(raw::LLVMGetInstructionParent(self.raw))
            {
                bad_call(format_args!("add_case: %{} is in another function", destination.name()));
            }
            raw::LLVMAddCase(self.raw, on_value.raw, destination.raw);
        }
    }

    /// How many cases this `switch` has, not counting the default.
    #[must_use]
    pub fn case_count(self) -> u32 {
        // SAFETY: the marker guarantees a `switch`, whose first successor is its default.
        unsafe { raw::LLVMGetNumSuccessors(self.raw) - 1 }
    }

    /// The value of the case at `index`.
    ///
    /// # Panics
    /// If there is no such case (D3).
    #[must_use]
    #[track_caller]
    pub fn case_value(self, index: u32) -> Value<'context, 'module, ConstantInteger> {
        let count = self.case_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("case_value: index {index} is out of bounds for {count} cases"));
        }
        // SAFETY: the index was just checked. LLVM's C API counts the default as successor 0, so case i is i + 1.
        unsafe { Value::from_raw(raw::LLVMGetSwitchCaseValue(self.raw, index + 1)) }
    }

    /// Changes the value of the case at `index`.
    ///
    /// # Panics
    /// If there is no such case, or the value isn't a constant integer of the switch's type (D3).
    #[track_caller]
    pub fn set_case_value<T>(self, index: u32, case_value: Value<'context, 'module, T>) {
        let count = self.case_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("set_case_value: index {index} is out of bounds for {count} cases"));
        }
        // SAFETY: the index was just checked, and so is the value's class and type before it is set.
        unsafe {
            if raw::LLVMIsAConstantInt(case_value.raw).is_null()
                || !checks::same_identity(
                    raw::LLVMTypeOf(raw::LLVMGetOperand(self.raw, 0)) as usize,
                    raw::LLVMTypeOf(case_value.raw) as usize,
                )
            {
                bad_call(format_args!(
                    "set_case_value: {} is not a constant of the switch's type",
                    case_value.describe()
                ));
            }
            raw::LLVMSetSwitchCaseValue(self.raw, index + 1, case_value.raw);
        }
    }
}

impl<'context, 'module> Value<'context, 'module, IndirectBranchInstruction> {
    /// Adds a block this branch can go to.
    ///
    /// # Panics
    /// If the block is in another function (D3).
    #[track_caller]
    pub fn add_destination(self, destination: Block<'context, 'module>) {
        // SAFETY: both are live for `'module`, and the parents of placed instructions and blocks exist.
        unsafe {
            if raw::LLVMGetBasicBlockParent(destination.raw)
                != raw::LLVMGetBasicBlockParent(raw::LLVMGetInstructionParent(self.raw))
            {
                bad_call(format_args!("add_destination: %{} is in another function", destination.name()));
            }
            raw::LLVMAddDestination(self.raw, destination.raw);
        }
    }
}

impl<'context, 'module> Value<'context, 'module, ConditionalBranchInstruction> {
    /// This branch's condition.
    #[must_use]
    pub fn condition(self) -> Value<'context, 'module, Integer<crate::types::Bits<1>>> {
        // SAFETY: the marker guarantees a conditional branch.
        unsafe { Value::from_raw(raw::LLVMGetCondition(self.raw)) }
    }

    /// Sets this branch's condition.
    ///
    /// # Panics
    /// If the condition belongs to another module (D3).
    #[track_caller]
    pub fn set_condition(self, condition: Value<'context, 'module, Integer<crate::types::Bits<1>>>) {
        // SAFETY: both are live; the shim only reads them.
        unsafe {
            if !shim::lyftar_values_share_module(self.raw, condition.raw) {
                bad_call(format_args!("set_condition: {} belongs to another module", condition.describe()));
            }
            raw::LLVMSetCondition(self.raw, condition.raw);
        }
    }
}

impl<'context, 'module, T: TerminatorMarker> Value<'context, 'module, T> {
    /// How many blocks this terminator can go to.
    #[must_use]
    pub fn successor_count(self) -> u32 {
        // SAFETY: the marker guarantees a terminator.
        unsafe { raw::LLVMGetNumSuccessors(self.raw) }
    }

    /// The block at `index` among those this terminator can go to.
    ///
    /// # Panics
    /// If there is no such successor (D3).
    #[must_use]
    #[track_caller]
    pub fn successor(self, index: u32) -> Block<'context, 'module> {
        let count = self.successor_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("successor: index {index} is out of bounds for {count} successors"));
        }
        // SAFETY: the index was just checked.
        unsafe { Block::from_raw(raw::LLVMGetSuccessor(self.raw, index)) }
    }

    /// Changes the block at `index` among those this terminator can go to.
    ///
    /// # Panics
    /// If there is no such successor, or the block is in another function (D3).
    #[track_caller]
    pub fn set_successor(self, index: u32, block: Block<'context, 'module>) {
        let count = self.successor_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("set_successor: index {index} is out of bounds for {count} successors"));
        }
        // SAFETY: the index was just checked, and the parents exist for placed instructions and blocks.
        unsafe {
            if raw::LLVMGetBasicBlockParent(block.raw)
                != raw::LLVMGetBasicBlockParent(raw::LLVMGetInstructionParent(self.raw))
            {
                bad_call(format_args!("set_successor: %{} is in another function", block.name()));
            }
            raw::LLVMSetSuccessor(self.raw, index, block.raw);
        }
    }
}

impl<'context, 'module, T: CallBaseMarker> Value<'context, 'module, T> {
    /// How many arguments this call passes.
    #[must_use]
    pub fn argument_count(self) -> u32 {
        // SAFETY: the marker guarantees a call of some kind.
        unsafe { raw::LLVMGetNumArgOperands(self.raw) }
    }

    /// The argument at `index`.
    ///
    /// # Panics
    /// If there is no such argument (D3).
    #[must_use]
    #[track_caller]
    pub fn argument(self, index: u32) -> Value<'context, 'module> {
        let count = self.argument_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("argument: index {index} is out of bounds for {count} arguments"));
        }
        // SAFETY: a call's arguments are its first operands, and the index was just checked.
        unsafe { Value::from_raw(raw::LLVMGetOperand(self.raw, index)) }
    }

    /// This call's calling convention.
    #[must_use]
    pub fn calling_convention(self) -> crate::functions::CallingConvention {
        // SAFETY: the marker guarantees a call of some kind.
        crate::functions::CallingConvention(unsafe { raw::LLVMGetInstructionCallConv(self.raw) })
    }

    /// Sets this call's calling convention.
    ///
    /// # Panics
    /// If LLVM can't hold it, past 1023 (D3).
    #[track_caller]
    pub fn set_calling_convention(self, convention: crate::functions::CallingConvention) {
        let number = convention.checked_number("set_calling_convention");
        // SAFETY: the marker guarantees a call of some kind, and the number was just checked.
        unsafe { raw::LLVMSetInstructionCallConv(self.raw, number) };
    }

    /// Sets the alignment of the attribute at a location of this call.
    ///
    /// # Panics
    /// If the alignment isn't a power of two (D3).
    #[track_caller]
    pub fn set_parameter_alignment_in_bytes(self, location: AttributeLocation, alignment_in_bytes: u32) {
        if !checks::is_power_of_two(alignment_in_bytes) {
            bad_call(format_args!("set_parameter_alignment_in_bytes: {alignment_in_bytes} is not a power of two"));
        }
        self.check_attribute_location("set_parameter_alignment_in_bytes", location);
        // SAFETY: the marker guarantees a call of some kind, and the alignment was just checked.
        unsafe { raw::LLVMSetInstrParamAlignment(self.raw, location.to_raw(), alignment_in_bytes) };
    }

    #[track_caller]
    fn check_attribute_location(self, operation: &str, location: AttributeLocation) {
        if let AttributeLocation::Parameter(index) = location {
            let count = self.argument_count();
            if !checks::index_in_bounds(index, count) {
                bad_call(format_args!("{operation}: parameter {index} is out of bounds for {count} arguments"));
            }
        }
    }

    /// Adds an attribute at a location of this call.
    ///
    /// # Panics
    /// If the location is a parameter this call doesn't have (D3).
    #[track_caller]
    pub fn add_attribute(self, location: AttributeLocation, attribute: Attribute<'context>) {
        self.check_attribute_location("add_attribute", location);
        // SAFETY: the location was just checked, and the attribute is live for `'context`.
        unsafe { raw::LLVMAddCallSiteAttribute(self.raw, location.to_raw(), attribute.raw) };
    }

    /// The attributes at a location of this call.
    ///
    /// # Panics
    /// If the location is a parameter this call doesn't have (D3).
    #[must_use]
    #[track_caller]
    pub fn attributes(self, location: AttributeLocation) -> Vec<Attribute<'context>> {
        self.check_attribute_location("attributes", location);
        // SAFETY: the location was just checked, and the buffer has room for all of them.
        unsafe {
            let count = raw::LLVMGetCallSiteAttributeCount(self.raw, location.to_raw());
            let mut attributes = vec![std::ptr::null_mut(); count as usize];
            raw::LLVMGetCallSiteAttributes(self.raw, location.to_raw(), attributes.as_mut_ptr());
            attributes.into_iter().map(|attribute| Attribute::from_raw(attribute)).collect()
        }
    }

    /// The attribute of an enum kind at a location of this call, if it has it.
    ///
    /// # Panics
    /// If the location is a parameter this call doesn't have (D3).
    #[must_use]
    #[track_caller]
    pub fn enum_attribute(
        self,
        location: AttributeLocation,
        kind: crate::attributes::AttributeKind,
    ) -> Option<Attribute<'context>> {
        self.check_attribute_location("enum_attribute", location);
        // SAFETY: the location was just checked.
        let found = unsafe { raw::LLVMGetCallSiteEnumAttribute(self.raw, location.to_raw(), kind.0) };
        // SAFETY: LLVM gave back a live attribute.
        (!found.is_null()).then(|| unsafe { Attribute::from_raw(found) })
    }

    /// The attribute with a string key at a location of this call, if it has it.
    ///
    /// # Panics
    /// If the location is a parameter this call doesn't have (D3).
    #[must_use]
    #[track_caller]
    pub fn string_attribute(self, location: AttributeLocation, key: &str) -> Option<Attribute<'context>> {
        self.check_attribute_location("string_attribute", location);
        let length = length_as_u32(key.len(), "string_attribute");
        // SAFETY: the location was just checked; LLVM reads `length` bytes of the key.
        let found =
            unsafe { raw::LLVMGetCallSiteStringAttribute(self.raw, location.to_raw(), key.as_ptr().cast(), length) };
        // SAFETY: LLVM gave back a live attribute.
        (!found.is_null()).then(|| unsafe { Attribute::from_raw(found) })
    }

    /// Removes the attribute of an enum kind at a location of this call.
    ///
    /// # Panics
    /// If the location is a parameter this call doesn't have (D3).
    #[track_caller]
    pub fn remove_enum_attribute(self, location: AttributeLocation, kind: crate::attributes::AttributeKind) {
        self.check_attribute_location("remove_enum_attribute", location);
        // SAFETY: the location was just checked.
        unsafe { raw::LLVMRemoveCallSiteEnumAttribute(self.raw, location.to_raw(), kind.0) };
    }

    /// Removes the attribute with a string key at a location of this call.
    ///
    /// # Panics
    /// If the location is a parameter this call doesn't have (D3).
    #[track_caller]
    pub fn remove_string_attribute(self, location: AttributeLocation, key: &str) {
        self.check_attribute_location("remove_string_attribute", location);
        let length = length_as_u32(key.len(), "remove_string_attribute");
        // SAFETY: the location was just checked; LLVM reads `length` bytes of the key.
        unsafe { raw::LLVMRemoveCallSiteStringAttribute(self.raw, location.to_raw(), key.as_ptr().cast(), length) };
    }

    /// The function type this call is made through.
    #[must_use]
    pub fn called_function_type(self) -> Type<'context> {
        // SAFETY: the marker guarantees a call of some kind.
        unsafe { Type::from_raw(raw::LLVMGetCalledFunctionType(self.raw)) }
    }

    /// What this call calls, which can be any pointer.
    #[must_use]
    pub fn called_value(self) -> Value<'context, 'module> {
        // SAFETY: the marker guarantees a call of some kind.
        unsafe { Value::from_raw(raw::LLVMGetCalledValue(self.raw)) }
    }

    /// The function this call calls directly, if it does: none for a call through a pointer, or through a function
    /// type other than the callee's own.
    #[must_use]
    pub fn called_function(self) -> Option<Value<'context, 'module, Function>> {
        let called = self.called_value().as_kind::<Function>()?;
        // SAFETY: a function has a value type, and it is live.
        let type_matches = unsafe { raw::LLVMGlobalGetValueType(called.raw) } == self.called_function_type().raw;
        type_matches.then_some(called)
    }

    /// Points this call at another function of the same type.
    ///
    /// # Panics
    /// If the function's type differs from this call's, or it belongs to another module (D3).
    #[track_caller]
    pub fn set_called_function(self, function: Value<'context, 'module, Function>) {
        // SAFETY: both are live for `'module`, and the shim only reads them until they are checked.
        unsafe {
            if raw::LLVMGlobalGetValueType(function.raw) != raw::LLVMGetCalledFunctionType(self.raw) {
                bad_call(format_args!(
                    "set_called_function: {} doesn't have the type this call is made through",
                    function.name()
                ));
            }
            if !shim::lyftar_values_share_module(self.raw, function.raw) {
                bad_call(format_args!("set_called_function: {} belongs to another module", function.name()));
            }
            shim::lyftar_call_set_called_function(self.raw, function.raw);
        }
    }

    /// Whether LLVM knows this call doesn't keep the pointer passed as argument `index`.
    ///
    /// # Panics
    /// If there is no such argument (D3).
    #[must_use]
    #[track_caller]
    pub fn does_not_capture(self, index: u32) -> bool {
        let count = self.argument_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("does_not_capture: index {index} is out of bounds for {count} arguments"));
        }
        // SAFETY: the index was just checked.
        unsafe { shim::lyftar_call_does_not_capture(self.raw, index) }
    }

    /// The operand bundles on this call.
    #[must_use]
    pub fn operand_bundles(self) -> Vec<crate::builder::OperandBundle<'context, 'module>> {
        // SAFETY: the marker guarantees a call of some kind; each index is below the count LLVM gave, and each bundle
        // copy is owned by the wrapper.
        unsafe {
            (0..raw::LLVMGetNumOperandBundles(self.raw))
                .map(|index| crate::builder::OperandBundle::from_raw(raw::LLVMGetOperandBundleAtIndex(self.raw, index)))
                .collect()
        }
    }
}

impl Value<'_, '_, crate::values::CallInstruction> {
    /// Whether this call is marked `tail`.
    #[must_use]
    pub fn is_tail_call(self) -> bool {
        // SAFETY: the marker guarantees a `call`.
        unsafe { raw::LLVMIsTailCall(self.raw) != 0 }
    }

    /// Sets whether this call is marked `tail`.
    pub fn set_tail_call(self, value: bool) {
        // SAFETY: the marker guarantees a `call`.
        unsafe { raw::LLVMSetTailCall(self.raw, i32::from(value)) };
    }

    /// This call's tail-call marker.
    #[must_use]
    pub fn tail_call_kind(self) -> TailCallKind {
        // SAFETY: the marker guarantees a `call`.
        TailCallKind::from_raw(unsafe { raw::LLVMGetTailCallKind(self.raw) })
    }

    /// Sets this call's tail-call marker.
    pub fn set_tail_call_kind(self, kind: TailCallKind) {
        // SAFETY: the marker guarantees a `call`.
        unsafe { raw::LLVMSetTailCallKind(self.raw, kind.to_raw()) };
    }
}

impl<'context, 'module> Value<'context, 'module, InvokeInstruction> {
    /// Where this invoke goes when the call returns.
    #[must_use]
    pub fn normal_destination(self) -> Block<'context, 'module> {
        // SAFETY: the marker guarantees an `invoke`.
        unsafe { Block::from_raw(raw::LLVMGetNormalDest(self.raw)) }
    }

    /// Where this invoke goes when the call unwinds.
    #[must_use]
    pub fn unwind_destination(self) -> Block<'context, 'module> {
        // SAFETY: the marker guarantees an `invoke`.
        unsafe { Block::from_raw(raw::LLVMGetUnwindDest(self.raw)) }
    }

    /// Sets where this invoke goes when the call returns.
    ///
    /// # Panics
    /// If the block is in another function (D3).
    #[track_caller]
    pub fn set_normal_destination(self, block: Block<'context, 'module>) {
        self.check_same_function("set_normal_destination", block);
        // SAFETY: the marker guarantees an `invoke`, and the block was just checked.
        unsafe { raw::LLVMSetNormalDest(self.raw, block.raw) };
    }

    /// Sets where this invoke goes when the call unwinds.
    ///
    /// # Panics
    /// If the block is in another function (D3).
    #[track_caller]
    pub fn set_unwind_destination(self, block: Block<'context, 'module>) {
        self.check_same_function("set_unwind_destination", block);
        // SAFETY: the marker guarantees an `invoke`, and the block was just checked.
        unsafe { raw::LLVMSetUnwindDest(self.raw, block.raw) };
    }
}

impl<'context, 'module, T: InstructionMarker> Value<'context, 'module, T> {
    #[track_caller]
    pub(crate) fn check_same_function(self, operation: &str, block: Block<'context, 'module>) {
        // SAFETY: both are live for `'module`, and placed.
        let same = unsafe {
            raw::LLVMGetBasicBlockParent(block.raw)
                == raw::LLVMGetBasicBlockParent(raw::LLVMGetInstructionParent(self.raw))
        };
        if !same {
            bad_call(format_args!("{operation}: %{} is in another function", block.name()));
        }
    }
}

impl<'context, 'module> Value<'context, 'module, CallBranchInstruction> {
    /// Where this call branch goes by default.
    #[must_use]
    pub fn default_destination(self) -> Block<'context, 'module> {
        // SAFETY: the marker guarantees a `callbr`.
        unsafe { Block::from_raw(raw::LLVMGetCallBrDefaultDest(self.raw)) }
    }

    /// How many indirect destinations this call branch has.
    #[must_use]
    pub fn indirect_destination_count(self) -> u32 {
        // SAFETY: the marker guarantees a `callbr`.
        unsafe { raw::LLVMGetCallBrNumIndirectDests(self.raw) }
    }

    /// The indirect destination at `index`.
    ///
    /// # Panics
    /// If there is no such destination (D3).
    #[must_use]
    #[track_caller]
    pub fn indirect_destination(self, index: u32) -> Block<'context, 'module> {
        let count = self.indirect_destination_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("indirect_destination: index {index} is out of bounds for {count} destinations"));
        }
        // SAFETY: the index was just checked.
        unsafe { Block::from_raw(raw::LLVMGetCallBrIndirectDest(self.raw, index)) }
    }
}

impl<'context, 'module> Value<'context, 'module, LandingPadInstruction> {
    /// How many clauses this landing pad has.
    #[must_use]
    pub fn clause_count(self) -> u32 {
        // SAFETY: the marker guarantees a `landingpad`.
        unsafe { raw::LLVMGetNumClauses(self.raw) }
    }

    /// The clause at `index`.
    ///
    /// # Panics
    /// If there is no such clause (D3).
    #[must_use]
    #[track_caller]
    pub fn clause(self, index: u32) -> Value<'context, 'module, crate::values::Constant> {
        let count = self.clause_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("clause: index {index} is out of bounds for {count} clauses"));
        }
        // SAFETY: the index was just checked, and a clause is a constant.
        unsafe { Value::from_raw(raw::LLVMGetClause(self.raw, index)) }
    }

    /// Adds a clause: a catch's type, or a filter's array.
    ///
    /// # Panics
    /// If the clause isn't a constant, or it belongs to another module (D3).
    #[track_caller]
    pub fn add_clause<T>(self, clause: Value<'context, 'module, T>) {
        if !clause.is_constant() {
            bad_call(format_args!("add_clause: {} is not a constant", clause.describe()));
        }
        // SAFETY: both live for `'module`; the shim only reads them.
        if !unsafe { shim::lyftar_values_share_module(self.raw, clause.raw) } {
            bad_call(format_args!("add_clause: {} belongs to another module", clause.describe()));
        }
        // SAFETY: just checked what LLVM requires.
        unsafe { raw::LLVMAddClause(self.raw, clause.raw) };
    }

    /// Whether this landing pad is a cleanup.
    #[must_use]
    pub fn is_cleanup(self) -> bool {
        // SAFETY: the marker guarantees a `landingpad`.
        unsafe { raw::LLVMIsCleanup(self.raw) != 0 }
    }

    /// Sets whether this landing pad is a cleanup.
    pub fn set_cleanup(self, value: bool) {
        // SAFETY: the marker guarantees a `landingpad`.
        unsafe { raw::LLVMSetCleanup(self.raw, i32::from(value)) };
    }
}

impl<'context, 'module> Value<'context, 'module, CatchSwitchInstruction> {
    /// Adds a block holding a `catchpad`.
    ///
    /// # Panics
    /// If the block is in another function (D3).
    #[track_caller]
    pub fn add_handler(self, destination: Block<'context, 'module>) {
        self.check_same_function("add_handler", destination);
        // SAFETY: the marker guarantees a `catchswitch`, and the block was just checked.
        unsafe { raw::LLVMAddHandler(self.raw, destination.raw) };
    }

    /// This `catchswitch`'s handlers.
    #[must_use]
    pub fn handlers(self) -> Vec<Block<'context, 'module>> {
        // SAFETY: the marker guarantees a `catchswitch`, and the buffer has room for all of them.
        unsafe {
            let count = raw::LLVMGetNumHandlers(self.raw);
            let mut handlers = vec![std::ptr::null_mut(); count as usize];
            raw::LLVMGetHandlers(self.raw, handlers.as_mut_ptr());
            handlers.into_iter().map(|handler| Block::from_raw(handler)).collect()
        }
    }
}

impl<'context, 'module> Value<'context, 'module, FuncletPadInstruction> {
    /// How many arguments this pad has.
    #[must_use]
    pub fn argument_count(self) -> u32 {
        // SAFETY: the marker guarantees a pad, which is what LLVM wants here.
        unsafe { raw::LLVMGetNumArgOperands(self.raw) }
    }

    /// The argument at `index`.
    ///
    /// # Panics
    /// If there is no such argument (D3).
    #[must_use]
    #[track_caller]
    pub fn argument(self, index: u32) -> Value<'context, 'module> {
        let count = self.argument_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("argument: index {index} is out of bounds for {count} arguments"));
        }
        // SAFETY: the index was just checked.
        unsafe { Value::from_raw(raw::LLVMGetArgOperand(self.raw, index)) }
    }

    /// Sets the argument at `index`.
    ///
    /// # Panics
    /// If there is no such argument, or it belongs to another module (D3).
    #[track_caller]
    pub fn set_argument<T>(self, index: u32, argument: Value<'context, 'module, T>) {
        let count = self.argument_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("set_argument: index {index} is out of bounds for {count} arguments"));
        }
        // SAFETY: the index was just checked; the shim only reads both values.
        unsafe {
            if !shim::lyftar_values_share_module(self.raw, argument.raw) {
                bad_call(format_args!("set_argument: {} belongs to another module", argument.describe()));
            }
            raw::LLVMSetArgOperand(self.raw, index, argument.raw);
        }
    }
}

impl<'context, 'module> Value<'context, 'module, CatchPadInstruction> {
    /// The `catchswitch` this pad belongs to.
    #[must_use]
    pub fn parent_catch_switch(self) -> Value<'context, 'module, CatchSwitchInstruction> {
        // SAFETY: the marker guarantees a `catchpad`.
        unsafe { Value::from_raw(raw::LLVMGetParentCatchSwitch(self.raw)) }
    }

    /// Sets the `catchswitch` this pad belongs to.
    ///
    /// # Panics
    /// If it is in another function (D3).
    #[track_caller]
    pub fn set_parent_catch_switch(self, catch_switch: Value<'context, 'module, CatchSwitchInstruction>) {
        self.check_same_function("set_parent_catch_switch", catch_switch.parent_block());
        // SAFETY: the marker guarantees a `catchpad`, and the switch was just checked.
        unsafe { raw::LLVMSetParentCatchSwitch(self.raw, catch_switch.raw) };
    }
}

impl Value<'_, '_, ExtractValueInstruction> {
    /// The indices this `extractvalue` goes through.
    #[must_use]
    pub fn indices(self) -> Vec<u32> {
        // SAFETY: the marker guarantees one of the classes LLVM takes here.
        unsafe { copy_indices(self.raw) }
    }
}

impl Value<'_, '_, InsertValueInstruction> {
    /// The indices this `insertvalue` goes through.
    #[must_use]
    pub fn indices(self) -> Vec<u32> {
        // SAFETY: the marker guarantees one of the classes LLVM takes here.
        unsafe { copy_indices(self.raw) }
    }
}

/// Copies the indices of an `extractvalue` or `insertvalue`.
///
/// # Safety
/// `raw` must be a live `extractvalue` or `insertvalue`.
unsafe fn copy_indices(raw_value: raw::LLVMValueRef) -> Vec<u32> {
    // SAFETY: the caller promises the class, and LLVM says how many indices the pointer has.
    unsafe {
        let count = raw::LLVMGetNumIndices(raw_value) as usize;
        std::slice::from_raw_parts(raw::LLVMGetIndices(raw_value), count).to_vec()
    }
}

impl Value<'_, '_, ShuffleVectorInstruction> {
    /// How many elements this shuffle's mask has.
    #[must_use]
    pub fn mask_element_count(self) -> u32 {
        // SAFETY: the marker guarantees a `shufflevector`.
        unsafe { raw::LLVMGetNumMaskElements(self.raw) }
    }

    /// The mask's element at `index`, or none where it is `poison`.
    ///
    /// # Panics
    /// If there is no such element (D3).
    #[must_use]
    #[track_caller]
    pub fn mask_value(self, index: u32) -> Option<u32> {
        let count = self.mask_element_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("mask_value: index {index} is out of bounds for {count} mask elements"));
        }
        // SAFETY: the index was just checked.
        let (element, undefined) = unsafe { (raw::LLVMGetMaskValue(self.raw, index), raw::LLVMGetUndefMaskElem()) };
        if element == undefined { None } else { u32::try_from(element).ok() }
    }
}

impl<'context, 'module> Value<'context, 'module, MemoryIntrinsic> {
    /// The operand that says how many bytes this writes.
    #[must_use]
    pub fn length_in_bytes(self) -> Value<'context, 'module> {
        // SAFETY: operand 2 of `llvm.memcpy`, `llvm.memmove` and `llvm.memset` is the length.
        unsafe { Value::from_raw(raw::LLVMGetOperand(self.raw, 2)) }
    }

    /// Whether this copy or fill is volatile.
    #[must_use]
    pub fn is_volatile_memory(self) -> bool {
        // SAFETY: operand 3 of these intrinsics is the constant `i1` volatile flag.
        unsafe { raw::LLVMIsNull(raw::LLVMGetOperand(self.raw, 3)) == 0 }
    }
}

impl Value<'_, '_, IntrinsicInstruction> {
    /// Whether this call is `llvm.lifetime.start` or `llvm.lifetime.end`.
    #[must_use]
    pub fn is_lifetime_start_or_end(self) -> bool {
        // SAFETY: the marker guarantees a call of an intrinsic.
        unsafe { shim::lyftar_intrinsic_is_lifetime_start_or_end(self.raw) }
    }
}

impl<W> Value<'_, '_, Integer<W>> {
    /// This integer's width.
    #[must_use]
    pub fn width_in_bits(self) -> u32 {
        // SAFETY: `Integer` guarantees an integer value, whose type is an integer type.
        unsafe { raw::LLVMGetIntTypeWidth(raw::LLVMTypeOf(self.raw)) }
    }
}
