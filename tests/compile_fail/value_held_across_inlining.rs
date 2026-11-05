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
        let mut module = context.create_module("m");
        let i64_type = context.get_type::<I64>();
        let function_type = context.function_type(i64_type.to_dynamic(), &[i64_type.to_dynamic()], false);
        let function = module.add_function("f", function_type);
        let block = function.append_block("entry");
        let builder = Builder::at_end_of(block);
        let parameter = function.parameter(0);
        let sum = builder.integer_add(parameter, parameter, "sum");
        let ret = builder.return_value(sum);
        let _ = ret;
        builder.position_before(ret);
        let call = builder.call(function, &[sum], "again");
        let choice = module.call_to_inline(call);
        drop(builder);
        let _ = module.inline_call(choice);
        let _ = sum.print_to_string();
    });
}
