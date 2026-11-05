//! LLVM contexts (D10).

use std::marker::PhantomData;

use crate::module::Module;
use crate::raw;
use crate::text::c_string;
use crate::types::Invariant;

/// An LLVM context: it owns the types and constants, and everything from it carries its `'context` lifetime.
///
/// It only exists inside [`with_context`], and that lifetime is invariant, so a value can't be used with another
/// context or taken out of the closure (D10).
#[derive(Clone, Copy)]
pub struct Context<'context> {
    pub(crate) raw: raw::LLVMContextRef,
    marker: Invariant<'context>,
}

// trace: D10
/// Disposes the context on the way out of [`with_context`], a panic included.
struct DisposeOnExit(raw::LLVMContextRef);

impl Drop for DisposeOnExit {
    fn drop(&mut self) {
        // SAFETY: `with_context` created this context, nothing else disposes it, and everything borrowing it carries a
        // `'context` that ended with the closure.
        unsafe { raw::LLVMContextDispose(self.0) };
    }
}

// trace: D10
/// Runs `body` with a new LLVM context, and disposes the context afterwards.
///
/// Plain data such as object file bytes can be returned; nothing carrying the context's lifetime can.
pub fn with_context<R>(body: impl for<'context> FnOnce(Context<'context>) -> R) -> R {
    // SAFETY: creating a context has no preconditions.
    let raw = unsafe { raw::LLVMContextCreate() };
    let _dispose = DisposeOnExit(raw);
    body(Context { raw, marker: PhantomData })
}

impl<'context> Context<'context> {
    /// Wraps a raw context.
    ///
    /// # Safety
    /// `raw` must be a live context that outlives `'context`.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMContextRef) -> Self {
        Context { raw, marker: PhantomData }
    }

    /// The raw context.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(self) -> raw::LLVMContextRef {
        self.raw
    }

    /// Creates an empty module in this context.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn create_module(self, name: &str) -> Module<'context> {
        let name = c_string(name, "create_module");
        // SAFETY: `name` is a valid C string for the call, and the context is live for `'context`.
        let raw = unsafe { raw::LLVMModuleCreateWithNameInContext(name.as_ptr(), self.raw) };
        // SAFETY: the module was just created, in this context, and is owned by the wrapper from here on.
        unsafe { Module::from_raw(raw) }
    }
}
