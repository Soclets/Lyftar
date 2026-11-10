//! A garbage collection strategy written in Rust, as a plugin for `opt`:
//! `opt -load-pass-plugin=libstrategy_plugin.dylib -passes=rewrite-statepoints-for-gc`.

// `pass_plugin!` is a compile error on Windows (D44) and with the static LLVM (D53).
#![cfg(not(any(target_os = "windows", lyftar_static_llvm)))]

use lyftar::Type;
use lyftar::garbage_collection::{GarbageCollectionStrategy, register_strategy};
use lyftar::passes::PassBuilder;

/// Pointers in address space 5 are the managed ones.
struct AddressSpaceFive;

impl GarbageCollectionStrategy for AddressSpaceFive {
    const NAME: &'static str = "address-space-five";
    const DESCRIPTION: &'static str = "manages the pointers in address space 5";

    fn manages_pointer(&self, pointer_type: Type<'_>) -> Option<bool> {
        Some(pointer_type.address_space() == 5)
    }
}

fn register(_builder: &mut PassBuilder<'static>) {
    register_strategy(AddressSpaceFive);
}

lyftar::pass_plugin!("address-space-five", "0.1.0", register);
