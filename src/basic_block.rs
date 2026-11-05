//! Basic blocks.

use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::raw;
use crate::text::{c_string, from_borrowed};
use crate::types::Invariant;
use crate::values::{BasicBlockAsValue, Function, Instruction, Terminator, Value};

/// A basic block. It borrows its module, as a value does (D11).
#[derive(Clone, Copy)]
#[repr(transparent)]
// trace: D35
pub struct BasicBlock<'context, 'module> {
    pub(crate) raw: raw::LLVMBasicBlockRef,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl PartialEq for BasicBlock<'_, '_> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}
impl Eq for BasicBlock<'_, '_> {}
impl std::hash::Hash for BasicBlock<'_, '_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}
impl std::fmt::Debug for BasicBlock<'_, '_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "%{}", self.name())
    }
}

impl<'context, 'module> BasicBlock<'context, 'module> {
    /// Wraps a raw block.
    ///
    /// # Safety
    /// `raw` must be a live block in a function, and stay live for `'module`.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMBasicBlockRef) -> Self {
        BasicBlock { raw, marker: PhantomData }
    }

    /// The raw block.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(self) -> raw::LLVMBasicBlockRef {
        self.raw
    }

    /// This block as a value.
    #[must_use]
    pub fn as_value(self) -> Value<'context, 'module, BasicBlockAsValue> {
        // SAFETY: the block is live for `'module`.
        unsafe { Value::from_raw(raw::LLVMBasicBlockAsValue(self.raw)) }
    }

    /// This block's name, empty if it has none.
    #[must_use]
    pub fn name(self) -> String {
        // SAFETY: the block is live for `'module`, and the name is copied right away.
        from_borrowed(unsafe { raw::LLVMGetBasicBlockName(self.raw) })
    }

    /// The function this block is in.
    #[must_use]
    pub fn parent(self) -> Value<'context, 'module, Function> {
        // SAFETY: a `BasicBlock` is always in a function, which is live for `'module`.
        unsafe { Value::from_raw(raw::LLVMGetBasicBlockParent(self.raw)) }
    }

    /// This block's terminator, if it has one yet.
    #[must_use]
    pub fn terminator(self) -> Option<Value<'context, 'module, Terminator>> {
        // SAFETY: the block is live for `'module`.
        let terminator = unsafe { raw::LLVMGetBasicBlockTerminator(self.raw) };
        // SAFETY: LLVM said it is the terminator, live for `'module`.
        (!terminator.is_null()).then(|| unsafe { Value::from_raw(terminator) })
    }

    /// The next block in the function.
    #[must_use]
    pub fn next(self) -> Option<BasicBlock<'context, 'module>> {
        // SAFETY: the block is live for `'module`.
        let next = unsafe { raw::LLVMGetNextBasicBlock(self.raw) };
        (!next.is_null()).then_some(BasicBlock { raw: next, marker: PhantomData })
    }

    /// The previous block in the function.
    #[must_use]
    pub fn previous(self) -> Option<BasicBlock<'context, 'module>> {
        // SAFETY: the block is live for `'module`.
        let previous = unsafe { raw::LLVMGetPreviousBasicBlock(self.raw) };
        (!previous.is_null()).then_some(BasicBlock { raw: previous, marker: PhantomData })
    }

    /// The first instruction in this block.
    #[must_use]
    pub fn first_instruction(self) -> Option<Value<'context, 'module, Instruction>> {
        // SAFETY: the block is live for `'module`.
        let first = unsafe { raw::LLVMGetFirstInstruction(self.raw) };
        // SAFETY: LLVM said it is an instruction, live for `'module`.
        (!first.is_null()).then(|| unsafe { Value::from_raw(first) })
    }

    /// The last instruction in this block.
    #[must_use]
    pub fn last_instruction(self) -> Option<Value<'context, 'module, Instruction>> {
        // SAFETY: the block is live for `'module`.
        let last = unsafe { raw::LLVMGetLastInstruction(self.raw) };
        // SAFETY: LLVM said it is an instruction, live for `'module`.
        (!last.is_null()).then(|| unsafe { Value::from_raw(last) })
    }

    /// The instructions in this block, in order.
    #[must_use]
    pub fn instructions(self) -> Instructions<'context, 'module> {
        Instructions { next: self.first_instruction() }
    }

    /// Adds a new block just before this one, in the same function.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn insert_basic_block_before(self, name: &str) -> BasicBlock<'context, 'module> {
        let name = c_string(name, "insert_basic_block_before");
        // SAFETY: the block is live, and `name` is a valid C string for the call.
        unsafe {
            let context = raw::LLVMGetValueContext(raw::LLVMBasicBlockAsValue(self.raw));
            BasicBlock::from_raw(raw::LLVMInsertBasicBlockInContext(context, self.raw, name.as_ptr()))
        }
    }

    /// Moves this block just before `position` in the function.
    ///
    /// # Panics
    /// If `position` is in another function (D3).
    #[track_caller]
    pub fn move_before(self, position: BasicBlock<'context, 'module>) {
        self.check_same_function("move_before", position);
        // SAFETY: both are live, and in the same function.
        unsafe { raw::LLVMMoveBasicBlockBefore(self.raw, position.raw) };
    }

    /// Moves this block just after `position` in the function.
    ///
    /// # Panics
    /// If `position` is in another function (D3).
    #[track_caller]
    pub fn move_after(self, position: BasicBlock<'context, 'module>) {
        self.check_same_function("move_after", position);
        // SAFETY: both are live, and in the same function.
        unsafe { raw::LLVMMoveBasicBlockAfter(self.raw, position.raw) };
    }

    #[track_caller]
    fn check_same_function(self, operation: &str, other: BasicBlock<'context, 'module>) {
        if self.parent() != other.parent() {
            bad_call(format_args!("{operation}: %{} and %{} are in different functions", self.name(), other.name()));
        }
    }
}

/// The instructions of a block, from [`BasicBlock::instructions`].
pub struct Instructions<'context, 'module> {
    next: Option<Value<'context, 'module, Instruction>>,
}

impl Instructions<'_, '_> {
    pub(crate) fn empty() -> Self {
        Instructions { next: None }
    }
}

impl<'context, 'module> Iterator for Instructions<'context, 'module> {
    type Item = Value<'context, 'module, Instruction>;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        self.next = current.next_instruction();
        Some(current)
    }
}
