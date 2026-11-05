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
        let module = context.module(&module);
        let i32_type = context.get_type::<I32>();
        let i64_type = context.get_type::<I64>();
        let callee = module.add_specific_function::<Function<I32, (I32, I32)>>("callee");
        let caller = module.add_specific_function::<Function<Void, ()>>("caller");
        let builder = Builder::at_end_of(caller.append_basic_block("entry"));
        let one = i32_type.constant_integer(1, false);
        let wide = i64_type.constant_integer(1, false);
        let _ = builder.call(callee, (one, wide), "");
    });
}
