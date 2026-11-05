//! Process-wide LLVM services, and a few small pieces of `Support.h`, `Core.h` and `ErrorHandling.h`.

use std::ffi::{CString, c_char, c_void};
use std::path::Path;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::error::Error;
use crate::raw;
use crate::text::{c_path, c_string, from_borrowed};

/// LLVM's version: major, minor, patch.
#[must_use]
pub fn version() -> (u32, u32, u32) {
    let (mut major, mut minor, mut patch) = (0, 0, 0);
    // SAFETY: LLVM writes three numbers.
    unsafe { raw::LLVMGetVersion(&raw mut major, &raw mut minor, &raw mut patch) };
    (major, minor, patch)
}

/// The version of debug metadata this LLVM reads and writes.
#[must_use]
pub fn debug_metadata_version() -> u32 {
    // SAFETY: takes nothing.
    unsafe { raw::LLVMDebugMetadataVersion() }
}

/// Whether LLVM is set up for use from several threads, which it always is when built with threads.
#[must_use]
pub fn is_multithreaded() -> bool {
    // SAFETY: takes nothing.
    unsafe { raw::LLVMIsMultithreaded() != 0 }
}

/// Loads a dynamic library for good, so that the JIT can find its symbols.
///
/// # Errors
/// If it couldn't be loaded.
pub fn load_library_permanently(path: &Path) -> Result<(), Error> {
    let raw_path = c_path(path)?;
    // SAFETY: a valid C string for the call.
    if unsafe { raw::LLVMLoadLibraryPermanently(raw_path.as_ptr()) } == 0 {
        Ok(())
    } else {
        Err(Error::new(format!("could not load {}", path.display())))
    }
}

/// The address of a symbol in the process or a library loaded for good, if it is there.
///
/// # Panics
/// If `name` has a NUL byte inside it (D3).
#[must_use]
#[track_caller]
pub fn search_for_address_of_symbol(name: &str) -> Option<*mut c_void> {
    let name = c_string(name, "search_for_address_of_symbol");
    // SAFETY: a valid C string for the call; the result is only a number until used, which is `unsafe`.
    let found = unsafe { raw::LLVMSearchForAddressOfSymbol(name.as_ptr()) };
    (!found.is_null()).then_some(found)
}

/// Makes a symbol visible to the JIT at an address.
///
/// # Panics
/// If `name` has a NUL byte inside it (D3).
#[track_caller]
pub fn add_symbol(name: &str, address: *mut c_void) {
    let name = c_string(name, "add_symbol");
    // SAFETY: LLVM copies the valid C string, and the address is only held until generated code uses it, which is `unsafe`.
    unsafe { raw::LLVMAddSymbol(name.as_ptr(), address) };
}

/// Gives LLVM command-line options, as `opt` takes them, for its own settings. The first is the program's name.
///
/// LLVM exits the process on an option it doesn't know.
///
/// # Panics
/// If a string has a NUL byte inside it (D3).
#[track_caller]
pub fn parse_command_line_options(arguments: &[&str], overview: &str) {
    let owned: Vec<CString> = arguments.iter().map(|item| c_string(item, "parse_command_line_options")).collect();
    let pointers: Vec<*const c_char> = owned.iter().map(|item| item.as_ptr()).collect();
    let overview = c_string(overview, "parse_command_line_options");
    let Ok(count) = i32::try_from(pointers.len()) else {
        crate::bad_call::bad_call(format_args!("parse_command_line_options: too many options"))
    };
    // SAFETY: the list and strings last for the call, and LLVM keeps none of them.
    unsafe { raw::LLVMParseCommandLineOptions(count, pointers.as_ptr(), overview.as_ptr()) };
}

/// Prints a stack trace if the process crashes.
pub fn enable_pretty_stack_trace() {
    // SAFETY: takes nothing.
    unsafe { raw::LLVMEnablePrettyStackTrace() };
}

static FATAL_ERROR_HANDLER: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

unsafe extern "C" fn call_fatal_error_handler(reason: *const c_char) {
    let handler = FATAL_ERROR_HANDLER.load(Ordering::Acquire);
    if handler.is_null() {
        return;
    }
    // SAFETY: only `install_fatal_error_handler` stores here, and only this function type.
    let handler = unsafe { std::mem::transmute::<*mut (), fn(&str)>(handler) };
    let reason = from_borrowed(reason);
    crate::abort_on_panic(|| handler(&reason));
}

/// Sets what LLVM tells before it exits on a fatal error. LLVM exits the process when the handler returns.
pub fn install_fatal_error_handler(handler: fn(&str)) {
    FATAL_ERROR_HANDLER.store(handler as *mut (), Ordering::Release);
    // SAFETY: the function matches LLVM's signature and lasts for good.
    unsafe { raw::LLVMInstallFatalErrorHandler(Some(call_fatal_error_handler)) };
}

/// Puts back LLVM's default for a fatal error.
pub fn reset_fatal_error_handler() {
    // SAFETY: takes nothing.
    unsafe { raw::LLVMResetFatalErrorHandler() };
}
