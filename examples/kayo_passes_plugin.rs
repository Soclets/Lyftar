//! kayo's two passes as one plugin, for `opt -load-pass-plugin` and `clang -fpass-plugin`:
//! `opt -load-pass-plugin=libkayo_passes_plugin.dylib -passes='function(heap-to-stack),recursive-inline'`.

// `pass_plugin!` is a compile error on Windows (D44) and with the static LLVM (D53).
#![cfg(not(any(target_os = "windows", lyftar_static_llvm)))]

#[path = "kayo/heap_to_stack.rs"]
mod heap_to_stack;
#[path = "kayo/recursive_inline.rs"]
mod recursive_inline;

use heap_to_stack::HeapToStack;
use lyftar::passes::PassBuilder;
use recursive_inline::RecursiveInline;

fn register(builder: &mut PassBuilder<'static>) {
    builder.register_function_pass("heap-to-stack", || HeapToStack);
    builder.register_module_pass("recursive-inline", || RecursiveInline { depth: 3, max_size: 1000, inlined: None });
}

lyftar::pass_plugin!("kayo-passes", "0.1.0", register);
