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
        let exclusive_first = context.exclusive_module(&mut first);
        let _ = exclusive_first.verify();
        let exclusive_second = context.exclusive_module(&mut second);
        let _ = exclusive_second.verify();
        let _ = (&mut first, &mut second, one);
    });
}
