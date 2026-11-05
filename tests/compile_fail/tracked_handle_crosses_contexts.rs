#![allow(unused_imports)]

use lyftar::enums::*;
use lyftar::passes::*;
use lyftar::target::*;
use lyftar::values::*;
use lyftar::*;

type I32 = Integer<Bits<32>>;
type I64 = Integer<Bits<64>>;
type F32 = Float<Ieee754Binary<Bits<32>>>;
type F64 = Float<Ieee754Binary<Bits<64>>>;

fn main() {
    with_context(|first| {
        let module = first.create_module("m");
        let function = module.add_function("f", first.function_type(first.void_type(), &[], false));
        let handle = module.track(function);
        with_context(|second| {
            let other = second.create_module("other");
            let _ = handle.get(&other);
        });
    });
}
