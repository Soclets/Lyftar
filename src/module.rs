//! Modules.

use crate::bad_call::bad_call;
use crate::raw;
use crate::text::{c_string, from_owned_message};
use crate::types::{Dynamic, Invariant, Type};
use crate::values::{Function, Value};
use std::marker::PhantomData;

/// A module, owning its functions, globals and instructions.
///
/// A value from a module borrows it, and deleting from it or running passes on it needs `&mut self`, so a value held
/// across either doesn't compile (D11).
pub struct Module<'context> {
    pub(crate) raw: raw::LLVMModuleRef,
    marker: Invariant<'context>,
}

impl Drop for Module<'_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the module, and everything borrowing it has ended.
        unsafe { raw::LLVMDisposeModule(self.raw) };
    }
}

impl<'context> Module<'context> {
    /// Takes ownership of a raw module.
    ///
    /// # Safety
    /// `raw` must be a live module of this context that nothing else owns.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMModuleRef) -> Self {
        Module { raw, marker: PhantomData }
    }

    /// The raw module.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(&self) -> raw::LLVMModuleRef {
        self.raw
    }

    /// Adds a function of the given function type.
    ///
    /// # Panics
    /// If `function_type` isn't a function type, or `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn add_function<'module>(
        &'module self,
        name: &str,
        function_type: Type<'context, Dynamic>,
    ) -> Value<'context, 'module, Function> {
        // SAFETY: the type is live for `'context`.
        if unsafe { raw::LLVMGetTypeKind(function_type.raw) } != raw::LLVMTypeKind::LLVMFunctionTypeKind {
            bad_call(format_args!("add_function: {} is not a function type", function_type.print_to_string()));
        }
        let name = c_string(name, "add_function");
        // SAFETY: the module is live, the type was just checked, and `name` is a valid C string for the call.
        unsafe { Value::from_raw(raw::LLVMAddFunction(self.raw, name.as_ptr(), function_type.raw)) }
    }

    /// Runs LLVM's module verifier, and gives back its report if the module is invalid.
    ///
    /// # Errors
    /// The verifier's report.
    pub fn verify(&self) -> Result<(), String> {
        let mut message = std::ptr::null_mut();
        // SAFETY: the module is live, and the verifier allocates the message for the caller to free.
        let failed = unsafe {
            raw::LLVMVerifyModule(self.raw, raw::LLVMVerifierFailureAction::LLVMReturnStatusAction, &raw mut message)
        };
        let report = from_owned_message(message);
        if failed == 0 { Ok(()) } else { Err(report) }
    }

    /// LLVM's text for this module.
    #[must_use]
    pub fn print_to_string(&self) -> String {
        // SAFETY: the module is live.
        from_owned_message(unsafe { raw::LLVMPrintModuleToString(self.raw) })
    }
}
