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
        context.exclusive_module(&mut first).link_in(second).unwrap();
        let _ = context.module(&second);
        second = context.create_module("again");
        let _ = (&mut first, &mut second, one);
    });
}
