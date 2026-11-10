#![allow(unused_imports)]

use lyftar::enums::*;
use lyftar::passes::*;
use lyftar::target::*;
use lyftar::values::*;
use lyftar::*;

type I32 = Integer<Bits<32>>;
type I64 = Integer<Bits<64>>;
type F32 = Float<Ieee754Binary<Bits<32>>>;

fn main() {
    with_context(|context| {
        let module = context.create_module("m");
        let module = context.module(&module);
        let function = module.add_function("f", context.function_type(context.void_type(), &[], false));
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let callee = context.function_type(context.void_type(), &[], false).inline_assembly("nop", "", true, false, InlineAssemblyDialect::Att, false);
        let _ = builder.call(callee, (), "");
    });
}
