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
        let held = context.constant_struct(&[one, one], false);
        context.exclusive_module(&mut first).run_passes("globaldce", None, &PassOptions::default()).unwrap();
        let _ = held.print_to_string();
        let _ = (&mut first, &mut second, one);
    });
}
