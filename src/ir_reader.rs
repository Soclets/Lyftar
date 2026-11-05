//! Reading LLVM IR as text or bitcode.

use crate::context::Context;
use crate::error::Error;
use crate::memory_buffer::MemoryBuffer;
use crate::module::OwnedModule;
use crate::shim;

impl<'context> Context<'context> {
    /// Reads a module from a buffer holding LLVM IR, as text or bitcode.
    ///
    /// # Errors
    /// What is wrong with the IR.
    pub fn parse_ir(&self, buffer: &MemoryBuffer<'_>) -> Result<OwnedModule<'context>, Error> {
        let mut module = std::ptr::null_mut();
        let mut message = std::ptr::null_mut();
        // SAFETY: both are live, and the shim leaves the buffer with the caller; it writes the module, or a message for the
        // caller to free.
        let failed = unsafe { shim::lyftar_parse_ir(self.raw, buffer.raw, &raw mut module, &raw mut message) };
        if failed != 0 {
            return Err(Error::from_owned_message(message));
        }
        // SAFETY: LLVM just made this module, in this context, for the caller to own.
        Ok(unsafe { OwnedModule::from_raw(module) })
    }

    /// Reads a module from text holding LLVM IR.
    ///
    /// # Errors
    /// What is wrong with the IR.
    pub fn parse_ir_text(&self, text: &str) -> Result<OwnedModule<'context>, Error> {
        self.parse_ir(&MemoryBuffer::from_bytes(text.as_bytes(), "ir"))
    }
}
