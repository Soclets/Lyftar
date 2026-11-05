//! Basic blocks.

use std::marker::PhantomData;

use crate::raw;
use crate::types::Invariant;

/// A basic block. It borrows its module, as a value does (D11).
#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Block<'context, 'module> {
    pub(crate) raw: raw::LLVMBasicBlockRef,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl PartialEq for Block<'_, '_> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}
impl Eq for Block<'_, '_> {}

impl Block<'_, '_> {
    /// Wraps a raw block.
    ///
    /// # Safety
    /// `raw` must be a live block in a function, and stay live for `'module`.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMBasicBlockRef) -> Self {
        Block { raw, marker: PhantomData }
    }

    /// The raw block.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(self) -> raw::LLVMBasicBlockRef {
        self.raw
    }
}
