//! Lyftar builds LLVM IR from safe Rust and writes LLVM passes in Rust, for the latest stable LLVM (D1, D2).
//!
//! All work happens inside [`with_context`] (D10). A [`Value`] says as much as the code knows (D4): a type written into
//! the Rust code is checked at `cargo build`, and one from the program being compiled is checked when the line runs, where
//! a bad request panics and names the problem (D3).

#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![warn(missing_docs)]
// Every getter would carry it, with no real risk of a mistake here.
#![allow(clippy::must_use_candidate)]

pub mod attributes;
mod bad_call;
pub mod block;
pub mod builder;
mod checks;
pub mod constants;
pub mod context;
pub mod enums;
pub mod functions;
pub mod instructions;
pub mod intrinsics;
pub mod metadata;
pub mod module;
pub mod raw;
mod shim;
mod text;
pub mod types;
pub mod values;

pub use block::Block;
pub use builder::Builder;
pub use context::{Context, with_context};
pub use module::Module;
pub use types::{
    Bits, BrainFloat, Dynamic, Float, Ieee754Binary, Integer, Pointer, PowerPcDoubleDouble, Type, X87Extended,
};
pub use values::{Function, Value};

// trace: D3
/// Runs `body`, and aborts the process if it panics: this is for Rust code that LLVM's C++ calls back into, where a panic
/// must not unwind through C++ frames. The panic hook has already printed the message by then.
pub(crate) fn abort_on_panic<R>(body: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(result) => result,
        Err(_) => std::process::abort(),
    }
}
