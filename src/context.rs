//! LLVM contexts (D10).

use std::marker::PhantomData;

use crate::enums::DiagnosticSeverity;
use crate::module::{Module, OwnedModule};
use crate::raw;
use crate::text::{c_string, from_owned_message};
use crate::types::Invariant;

// trace: D39, D42, D43
/// An LLVM context: it owns the types and constants, and everything from it carries its `'context` lifetime.
///
/// It only exists inside [`with_context`], and that lifetime is invariant, so a value can't be used with another
/// context or taken out of the closure (D10).
///
/// Modules live inside it. [`Context::module`] gives a module to read and build, and every value borrows the context from
/// there. [`Context::exclusive_module`] needs the context exclusively, so no value from any module can be held across what it
/// allows, such as running passes. Types and plain data borrow nothing, and survive all of it.
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
pub fn with_context<R>(body: impl for<'context> FnOnce(&mut Context<'context>) -> R) -> R {
    // Anything in a context can reach LLVM's target registry, such as parsing a module's inline assembly.
    crate::target::set_up_targets();
    // SAFETY: creating a context has no preconditions.
    let raw = unsafe { raw::LLVMContextCreate() };
    let _dispose = DisposeOnExit(raw);
    body(&mut Context { raw, marker: PhantomData })
}

/// A diagnostic LLVM reports through a context.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    /// How serious it is.
    pub severity: DiagnosticSeverity,
    /// What it says.
    pub description: String,
}

unsafe extern "C" fn call_diagnostic_handler<F: FnMut(Diagnostic)>(
    information: raw::LLVMDiagnosticInfoRef,
    handler: *mut std::ffi::c_void,
) {
    // SAFETY: LLVM passes back the pointer given with this function, which `set_diagnostic_handler` leaked, and the
    // information is live for this call.
    let (handler, severity, description) = unsafe {
        (
            &mut *handler.cast::<F>(),
            DiagnosticSeverity::from_raw(raw::LLVMGetDiagInfoSeverity(information)),
            from_owned_message(raw::LLVMGetDiagInfoDescription(information)),
        )
    };
    crate::abort_on_panic(|| handler(Diagnostic { severity, description }));
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
    pub unsafe fn as_raw(&self) -> raw::LLVMContextRef {
        self.raw
    }

    /// A module, to read and build. Everything from it borrows this context and the handle, so nothing from it can be held
    /// across a call to [`Context::exclusive_module`] on any module, or the handle being dropped or moved.
    #[must_use]
    pub fn module<'borrow>(&'borrow self, module: &'borrow OwnedModule<'context>) -> &'borrow Module<'context> {
        &module.module
    }

    /// A module, for what can delete things: running passes, emitting code, erasing and replacing. It needs this context
    /// exclusively, since LLVM can delete a constant that another module shares (D42).
    #[must_use]
    pub fn exclusive_module<'borrow>(
        &'borrow mut self,
        module: &'borrow mut OwnedModule<'context>,
    ) -> &'borrow mut Module<'context> {
        &mut module.module
    }

    /// Creates an empty module in this context.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn create_module(&self, name: &str) -> OwnedModule<'context> {
        let name = c_string(name, "create_module");
        // SAFETY: `name` is a valid C string for the call, and the context is live for `'context`.
        let raw = unsafe { raw::LLVMModuleCreateWithNameInContext(name.as_ptr(), self.raw) };
        // SAFETY: the module was just created, in this context, and is owned by the wrapper from here on.
        unsafe { OwnedModule::from_raw(raw) }
    }

    /// Whether this context drops the names of values, which saves memory and time.
    #[must_use]
    pub fn discards_value_names(&self) -> bool {
        // SAFETY: the context is live for `'context`.
        unsafe { raw::LLVMContextShouldDiscardValueNames(self.raw) != 0 }
    }

    /// Sets whether this context drops the names of values, which saves memory and time.
    pub fn set_discard_value_names(&self, discard: bool) {
        // SAFETY: the context is live for `'context`.
        unsafe { raw::LLVMContextSetDiscardValueNames(self.raw, i32::from(discard)) };
    }

    /// Sets what happens to a diagnostic LLVM reports through this context. A panic inside the handler aborts, since it
    /// is running inside LLVM (D3).
    ///
    /// The handler lives until the context goes, so setting one again leaks the old one.
    pub fn set_diagnostic_handler<F: FnMut(Diagnostic) + 'context>(&self, handler: F) {
        let handler = Box::into_raw(Box::new(handler));
        // SAFETY: the function matches LLVM's signature, and `handler` stays valid for as long as the context does, since
        // it is leaked. The context is what calls it, and the closure's `'context` outlives the context's own use of it.
        unsafe {
            raw::LLVMContextSetDiagnosticHandler(self.raw, Some(call_diagnostic_handler::<F>), handler.cast());
        }
    }
}
