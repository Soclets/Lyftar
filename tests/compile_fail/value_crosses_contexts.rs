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
        let i64_first = first.get_type::<I64>();
        let function = module.add_function("f", first.function_type(first.void_type(), &[], false));
        let builder = Builder::at_end_of(function.append_block("entry"));
        with_context(|second| {
            let from_second = second.get_type::<I64>().constant_integer(1, false);
            let _ = builder.integer_add(i64_first.constant_integer(1, false), from_second, "");
        });
    });
}
