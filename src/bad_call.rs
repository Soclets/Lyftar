//! How a bug in the compiler using Lyftar stops: a panic at the call that names the problem (D3).

use std::fmt;

// trace: D3
/// Panics with `message`, reporting the caller's line.
#[cold]
#[inline(never)]
#[track_caller]
#[allow(clippy::panic)]
pub(crate) fn bad_call(message: fmt::Arguments<'_>) -> ! {
    panic!("{message}")
}
