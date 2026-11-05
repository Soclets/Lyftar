//! What can really fail when the compiler runs, such as reading a bitcode file, returns this (D3).

use crate::raw;
use crate::text;
use std::ffi::c_char;

/// Something that really failed at run time, with LLVM's message or Lyftar's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    message: String,
}

impl Error {
    /// An error with this message.
    pub fn new(message: impl Into<String>) -> Self {
        Error { message: message.into() }
    }

    /// What went wrong.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Turns an `LLVMErrorRef` into a result, consuming it.
    pub(crate) fn from_raw_result(raw_error: raw::LLVMErrorRef) -> Result<(), Error> {
        if raw_error.is_null() {
            return Ok(());
        }
        // SAFETY: LLVM gave back a live error, consumed by this call; the message it makes is copied and then freed.
        let message = unsafe {
            let message = raw::LLVMGetErrorMessage(raw_error);
            let copied = text::from_borrowed(message);
            raw::LLVMDisposeErrorMessage(message);
            copied
        };
        Err(Error { message })
    }

    /// An error from a message LLVM allocated, freeing it.
    pub(crate) fn from_owned_message(message: *mut c_char) -> Self {
        Error { message: text::from_owned_message(message) }
    }

    /// Hands this error to LLVM, which takes ownership of what is returned.
    pub(crate) fn into_raw(self) -> raw::LLVMErrorRef {
        // A message with a NUL in it is cut there, and this path never panics, since it runs inside LLVM.
        let cut = self.message.split('\0').next().unwrap_or_default().to_string();
        let message = std::ffi::CString::new(cut).unwrap_or_default();
        // SAFETY: the message is a valid C string, which LLVM copies.
        unsafe { raw::LLVMCreateStringError(message.as_ptr()) }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}
