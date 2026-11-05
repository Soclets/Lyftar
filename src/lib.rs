//! Lyftar builds LLVM IR from safe Rust and writes LLVM passes in Rust, for the latest stable LLVM (D1, D2).
//!
//! All work happens inside [`with_context`] (D10).

#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![warn(missing_docs)]

mod bad_call;
pub mod block;
pub mod builder;
mod checks;
pub mod context;
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
