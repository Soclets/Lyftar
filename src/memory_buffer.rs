//! Bytes held by LLVM.

use std::marker::PhantomData;
use std::path::Path;

use crate::error::Error;
use crate::raw;
use crate::text::{c_path, c_string};

/// Bytes LLVM reads from or writes to. A buffer from [`MemoryBuffer::from_bytes_no_copy`] borrows its bytes for `'data`.
pub struct MemoryBuffer<'data> {
    pub(crate) raw: raw::LLVMMemoryBufferRef,
    marker: PhantomData<&'data [u8]>,
}

impl Drop for MemoryBuffer<'_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the buffer.
        unsafe { raw::LLVMDisposeMemoryBuffer(self.raw) };
    }
}

impl<'data> MemoryBuffer<'data> {
    /// Takes ownership of a raw buffer.
    ///
    /// # Safety
    /// `raw` must be a live buffer that nothing else owns, and what it points at must last for `'data`.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMMemoryBufferRef) -> Self {
        MemoryBuffer { raw, marker: PhantomData }
    }

    /// The raw buffer.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(&self) -> raw::LLVMMemoryBufferRef {
        self.raw
    }

    pub(crate) fn into_raw(self) -> raw::LLVMMemoryBufferRef {
        let raw = self.raw;
        std::mem::forget(self);
        raw
    }

    /// A buffer that uses `bytes` directly, with no copy.
    ///
    /// LLVM's parsers want a NUL after the end, so this only suits readers that don't, such as object files.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn from_bytes_no_copy(bytes: &'data [u8], name: &str) -> Self {
        let name = c_string(name, "from_bytes_no_copy");
        // SAFETY: the bytes last for `'data`, the name is a valid C string for the call, and no NUL is promised.
        let raw =
            unsafe { raw::LLVMCreateMemoryBufferWithMemoryRange(bytes.as_ptr().cast(), bytes.len(), name.as_ptr(), 0) };
        MemoryBuffer { raw, marker: PhantomData }
    }

    /// The bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: the buffer is live, and these calls say where its bytes are and how many. A slice can't be built from a
        // null pointer, so an empty buffer gives an empty slice directly.
        unsafe {
            let start = raw::LLVMGetBufferStart(self.raw);
            let size = raw::LLVMGetBufferSize(self.raw);
            if start.is_null() || size == 0 {
                return &[];
            }
            std::slice::from_raw_parts(start.cast::<u8>(), size)
        }
    }
}

impl MemoryBuffer<'static> {
    /// The contents of a file.
    ///
    /// # Errors
    /// What stopped the file being read.
    pub fn from_file(path: &Path) -> Result<Self, Error> {
        let path = c_path(path)?;
        let mut buffer = std::ptr::null_mut();
        let mut message = std::ptr::null_mut();
        // SAFETY: the path is a valid C string; LLVM writes the buffer, or a message for the caller to free.
        let failed =
            unsafe { raw::LLVMCreateMemoryBufferWithContentsOfFile(path.as_ptr(), &raw mut buffer, &raw mut message) };
        if failed != 0 {
            return Err(Error::from_owned_message(message));
        }
        Ok(MemoryBuffer { raw: buffer, marker: PhantomData })
    }

    /// Everything on standard input.
    ///
    /// # Errors
    /// What stopped it being read.
    pub fn from_standard_input() -> Result<Self, Error> {
        let mut buffer = std::ptr::null_mut();
        let mut message = std::ptr::null_mut();
        // SAFETY: LLVM writes the buffer, or a message for the caller to free.
        let failed = unsafe { raw::LLVMCreateMemoryBufferWithSTDIN(&raw mut buffer, &raw mut message) };
        if failed != 0 {
            return Err(Error::from_owned_message(message));
        }
        Ok(MemoryBuffer { raw: buffer, marker: PhantomData })
    }

    /// A copy of `bytes`.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn from_bytes(bytes: &[u8], name: &str) -> Self {
        let name = c_string(name, "from_bytes");
        // SAFETY: LLVM copies the bytes, and the name is a valid C string for the call.
        let raw = unsafe {
            raw::LLVMCreateMemoryBufferWithMemoryRangeCopy(bytes.as_ptr().cast(), bytes.len(), name.as_ptr())
        };
        MemoryBuffer { raw, marker: PhantomData }
    }
}
