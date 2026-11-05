#![allow(unused_imports)]

use lyftar::enums::*;
use lyftar::passes::*;
use lyftar::target::*;
use lyftar::values::*;
use lyftar::*;

type B8 = Byte<Bits<8>>;
type B16 = Byte<Bits<16>>;
type I8 = Integer<Bits<8>>;

fn main() {
    with_context(|context| {
        let module = context.create_module("m");
        let b8_type = context.get_type::<B8>();
        let b16_type = context.get_type::<B16>();
        let i8_type = context.get_type::<I8>();
        let function = module.add_function("f", context.function_type(context.void_type(), &[], false));
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let condition = context.get_type::<Integer<Bits<1>>>().constant_integer(1, false);
        let _ = builder.integer_add(b8_type.constant_byte(1), b8_type.constant_byte(2), "");
        let _ = (b16_type, i8_type, condition);
    });
}
