#![allow(unused_imports)]

use lyftar::enums::*;
use lyftar::passes::*;
use lyftar::target::*;
use lyftar::values::*;
use lyftar::*;

type I1 = Integer<Bits<1>>;
type I32 = Integer<Bits<32>>;
type I64 = Integer<Bits<64>>;

fn main() {
    with_context(|context| {
        let module = context.create_module("m");
        let i32_type = context.get_type::<I32>();
        let i64_type = context.get_type::<I64>();
        let function = module.add_specific_function::<Function<Void, ()>>("f");
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let condition = context.get_type::<I1>().constant_integer(1, false);
        let short = context.get_type::<Array<I32, Elements<2>>>().constant_null();
        let long = context.get_type::<Array<I32, Elements<2>>>().constant_null();
        let _ = builder.select(condition, short, long, "");
        let _ = (i32_type, i64_type);
    });
}
