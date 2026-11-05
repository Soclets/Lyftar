#![allow(unused_imports)]

use lyftar::enums::*;
use lyftar::jit::*;
use lyftar::passes::*;
use lyftar::values::*;
use lyftar::*;

type I32 = Integer<Bits<32>>;

fn main() {
    with_context(|context| {
        let one = context.get_type::<I32>().constant_integer(1, false);
        let mut first = context.create_module("first");
        let mut second = context.create_module("second");
        let parsed = context.parse_ir_text("!named = !{!0}\n!0 = !{ { i32, i32 } { i32 1, i32 1 } }").unwrap();
        let named = context.module(&parsed).named_metadata("named").unwrap();
        let read = named.operands()[0].node_operands()[0].unwrap();
        context.exclusive_module(&mut second).run_passes("globaldce", None, &PassOptions::default()).unwrap();
        let _ = read.print_to_string();
        let _ = (&mut first, &mut second, one);
    });
}
