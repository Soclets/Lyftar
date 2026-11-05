//! Reading and writing LLVM bitcode.

use std::path::Path;

use crate::context::Context;
use crate::error::Error;
use crate::memory_buffer::MemoryBuffer;
use crate::module::Module;
use crate::raw;
use crate::text::c_path;

impl<'context> Context<'context> {
    /// Reads a whole module from bitcode.
    ///
    /// # Errors
    /// What is wrong with the bitcode.
    pub fn parse_bitcode(self, buffer: &MemoryBuffer<'_>) -> Result<Module<'context>, Error> {
        let mut module = std::ptr::null_mut();
        let mut message = std::ptr::null_mut();
        // SAFETY: both are live; LLVM writes the module, or a message for the caller to free. This form reports errors
        // as text, where the newer one reports through the context, whose default handler exits the process.
        let failed = unsafe { raw::LLVMParseBitcodeInContext(self.raw, buffer.raw, &raw mut module, &raw mut message) };
        if failed != 0 {
            return Err(Error::from_owned_message(message));
        }
        // SAFETY: LLVM just made this module, in this context, for the caller to own.
        Ok(unsafe { Module::from_raw(module) })
    }

    /// Reads a module from bitcode lazily: each function's body comes in when first asked for. The module owns the
    /// buffer from here on.
    ///
    /// # Errors
    /// What is wrong with the bitcode.
    pub fn lazy_bitcode_module(self, buffer: MemoryBuffer<'static>) -> Result<Module<'context>, Error> {
        let raw_buffer = buffer.into_raw();
        let mut module = std::ptr::null_mut();
        let mut message = std::ptr::null_mut();
        // SAFETY: both are live. On success the module owns the buffer; on failure LLVM gives it back unfreed. This form
        // reports errors as text, where the newer one reports through the context, whose default handler exits the process.
        let failed =
            unsafe { raw::LLVMGetBitcodeModuleInContext(self.raw, raw_buffer, &raw mut module, &raw mut message) };
        if failed != 0 {
            // SAFETY: the failure left the buffer to the caller.
            unsafe { raw::LLVMDisposeMemoryBuffer(raw_buffer) };
            return Err(Error::from_owned_message(message));
        }
        // SAFETY: LLVM just made this module, in this context, for the caller to own.
        Ok(unsafe { Module::from_raw(module) })
    }
}

impl Module<'_> {
    // trace: D11
    /// Reads in the bodies of everything a lazy module hasn't yet. This changes the module in ways that can't be done with
    /// values held, so it needs exclusive access.
    ///
    /// # Errors
    /// What is wrong with the bitcode.
    pub fn materialize_all(&mut self) -> Result<(), Error> {
        self.bump_generation();
        // SAFETY: the module is held exclusively.
        Error::from_raw_result(unsafe { crate::shim::lyftar_module_materialize_all(self.raw) })
    }

    /// Writes this module as bitcode to a file.
    ///
    /// # Errors
    /// If the file couldn't be written.
    pub fn write_bitcode_to_file(&self, path: &Path) -> Result<(), Error> {
        let raw_path = c_path(path)?;
        // SAFETY: the module is live, and the path is a valid C string for the call.
        let failed = unsafe { raw::LLVMWriteBitcodeToFile(self.raw, raw_path.as_ptr()) };
        if failed == 0 { Ok(()) } else { Err(Error::new(format!("could not write bitcode to {}", path.display()))) }
    }

    /// This module as bitcode in memory.
    #[must_use]
    pub fn write_bitcode_to_memory_buffer(&self) -> MemoryBuffer<'static> {
        // SAFETY: the module is live; LLVM makes a new buffer for the caller to own.
        unsafe { MemoryBuffer::from_raw(raw::LLVMWriteBitcodeToMemoryBuffer(self.raw)) }
    }
}
