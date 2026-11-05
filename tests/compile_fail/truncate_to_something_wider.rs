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
    with_context(|context| {
        let module = context.create_module("m");
        let module = context.module(&module);
        let i32_type = context.get_type::<I32>();
        let i64_type = context.get_type::<I64>();
        let f32_type = context.get_type::<F32>();
        let f64_type = context.get_type::<F64>();
        let function = module.add_function("f", context.function_type(context.void_type(), &[], false));
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let _ = builder.truncate(i32_type.constant_integer(1, false), i64_type, "");
        let _ = (f32_type, f64_type);
    });
}
