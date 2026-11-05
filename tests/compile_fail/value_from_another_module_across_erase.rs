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
        let held = context.module(&first).add_global("g", one.get_type());
        let entry = {
            let module = context.module(&second);
            module.value_to_erase(module.add_global("h", one.get_type()))
        };
        context.exclusive_module(&mut second).erase(&[entry]);
        let _ = held.name();
        let _ = (&mut first, &mut second, one);
    });
}
