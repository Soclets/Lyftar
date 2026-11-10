//! Linking, with lld inside Lyftar, so a compiler built on it needs no linker installed (D49, D52).
//!
//! Links run one at a time in a process, since two at once crash lld. A link that fails is an error, and lld can run again after
//! one; if it ever says it can't, every later link is an error that says so.

use std::ffi::{CString, c_char};
use std::sync::Mutex;

use crate::error::Error;
use crate::text::c_string;

/// Which of lld's linkers, by the format it makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LldFlavor {
    /// ELF, as `ld.lld`: Linux and bare metal.
    Gnu,
    /// Mach-O, as `ld64.lld`: macOS.
    Darwin,
}

impl LldFlavor {
    /// The name lld's own executable has for this flavour, which lld reads its flavour from.
    fn program_name(self) -> &'static str {
        match self {
            LldFlavor::Gnu => "ld.lld",
            LldFlavor::Darwin => "ld64.lld",
        }
    }
}

/// Whether lld said it can't run again, after which no link in this process can.
static LLD: Mutex<bool> = Mutex::new(false);

// trace: D52
/// Runs lld with its own arguments, those of `ld.lld` or `ld64.lld` after the program's name, and gives back what it printed.
/// For what [`link`](crate::linking) doesn't cover.
///
/// # Errors
/// What lld printed, if the link failed, or why it can't run: this Lyftar's LLVM came without lld, or lld can't run again in
/// this process.
///
/// # Panics
/// If an argument has a NUL byte inside it (D3).
#[track_caller]
pub fn lld(flavor: LldFlavor, arguments: &[&str]) -> Result<String, Error> {
    let mut owned: Vec<CString> = Vec::with_capacity(arguments.len() + 1);
    owned.push(c_string(flavor.program_name(), "lld"));
    owned.extend(arguments.iter().map(|argument| c_string(argument, "lld")));
    let pointers: Vec<*const c_char> = owned.iter().map(|argument| argument.as_ptr()).collect();
    // A poisoned lock only means another link panicked in Rust, outside lld, which leaves it as it was.
    let mut cannot_run_again = LLD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if *cannot_run_again {
        return Err(Error::new("lld can't run again in this process, since an earlier link left it unable to"));
    }
    run_lld(&pointers, &mut cannot_run_again)
}

#[cfg(lyftar_has_lld)]
fn run_lld(arguments: &[*const c_char], cannot_run_again: &mut bool) -> Result<String, Error> {
    let mut output = std::ptr::null_mut();
    let mut can_run_again = true;
    // SAFETY: the arguments are C strings that outlive the call, the lock is held, and lld writes both outputs.
    let code = unsafe {
        crate::shim::lyftar_lld(arguments.as_ptr(), arguments.len(), &raw mut output, &raw mut can_run_again)
    };
    *cannot_run_again = !can_run_again;
    let printed = Error::from_owned_message(output).message().to_string();
    if code == 0 { Ok(printed) } else { Err(Error::new(printed)) }
}

#[cfg(not(lyftar_has_lld))]
fn run_lld(_arguments: &[*const c_char], _cannot_run_again: &mut bool) -> Result<String, Error> {
    Err(Error::new(
        "this Lyftar was built with an LLVM that has no lld libraries; Lyftar's own static LLVM has them (D50)",
    ))
}
