//! Lyftar builds LLVM IR from safe Rust and writes LLVM passes in Rust, for the latest stable LLVM (D1, D2).
//!
//! All work happens inside [`with_context`] (D10). A [`Value`] says as much as the code knows (D4): a type written into
//! the Rust code is checked at `cargo build`, and one from the program being compiled is checked when the line runs, where
//! a bad request panics and names the problem (D3).
//!
//! Modules live inside the context (D42, D43). You own an [`OwnedModule`], [`Context::module`] gives it to read and build, and
//! [`Context::exclusive_module`] gives it for what can delete things. Every value borrows the context, so one held across an
//! exclusive call on any module doesn't compile. Types and plain data borrow nothing, and survive.

#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![warn(missing_docs)]
// Every getter would carry it, with no real risk of a mistake here.
#![allow(clippy::must_use_candidate)]

// trace: D16
// One crate, whose modules follow how LLVM divides its C API.
//
// trace: D5, D8
// Every public name is in full words, and every number says its unit.
pub mod attributes;
mod bad_call;
pub mod basic_block;
pub mod bitcode;
pub mod blake3;
pub mod builder;
mod checks;
pub mod constants;
pub mod context;
pub mod debug_info;
pub mod disassembler;
pub mod enums;
pub mod error;
pub mod functions;
pub mod garbage_collection;
pub mod instructions;
pub mod intrinsics;
pub mod ir_reader;
pub mod jit;
pub mod linker;
pub mod linking;
pub mod lto;
pub mod memory_buffer;
pub mod metadata;
pub mod module;
pub mod object;
pub mod passes;
pub mod raw;
pub mod remarks;
#[doc(hidden)]
pub mod shim;
pub mod support;
pub mod target;
mod text;
pub mod types;
pub mod values;

pub use basic_block::BasicBlock;
pub use builder::Builder;
pub use context::{Context, with_context};
pub use linking::lld;
pub use module::{Module, OwnedModule};
pub use types::{
    AddressSpace, Array, Bits, BrainFloat, Byte, Dynamic, Elements, Float, Ieee754Binary, Integer, MinimumElements,
    PackedStruct, Pointer, PowerPcDoubleDouble, ScalableVector, Struct, Type, Vector, Void, X87Extended,
};
pub use values::{Function, Value, VariadicFunction};

// trace: D3
/// Runs `body`, and aborts the process if it panics: this is for Rust code that LLVM's C++ calls back into, where a panic
/// must not unwind through C++ frames. The panic hook has already printed the message by then.
pub(crate) fn abort_on_panic<R>(body: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(result) => result,
        Err(_) => std::process::abort(),
    }
}

/// The README's Rust compiles, so it can't drift from the API.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;
