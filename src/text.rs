//! Moving text across the C API.

use std::ffi::{CStr, CString, c_char};
use std::path::Path;

use crate::bad_call::bad_call;
use crate::raw;

// trace: D3
/// `text` as a C string. A NUL inside a name is a bug in the compiler using Lyftar, since LLVM would silently cut the
/// name there.
#[track_caller]
pub(crate) fn c_string(text: &str, what: &str) -> CString {
    match CString::new(text) {
        Ok(converted) => converted,
        Err(_) => bad_call(format_args!("{what}: {text:?} has a NUL byte inside it")),
    }
}

/// `path` as a C string, or an error for a path that has a NUL in it, which comes from the running program's input.
pub(crate) fn c_path(path: &Path) -> Result<CString, crate::error::Error> {
    CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| crate::error::Error::new(format!("{} has a NUL byte in it", path.display())))
}

/// Copies an LLVM string of known length.
pub(crate) fn from_pointer_and_length(pointer: *const c_char, length: usize) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: LLVM returned this pointer and length together, and the bytes live at least until this call returns,
    // since nothing here calls back into LLVM.
    let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) };
    String::from_utf8_lossy(bytes).into_owned()
}

/// Copies a NUL-terminated string that LLVM still owns.
pub(crate) fn from_borrowed(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: LLVM returned a NUL-terminated string that lives at least until this call returns.
    unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned()
}

/// Copies a message LLVM allocated with `LLVMCreateMessage`, and frees it.
pub(crate) fn from_owned_message(pointer: *mut c_char) -> String {
    let text = from_borrowed(pointer);
    if !pointer.is_null() {
        // SAFETY: LLVM allocated this message for the caller to free, and nothing uses it after this.
        unsafe { raw::LLVMDisposeMessage(pointer) };
    }
    text
}
