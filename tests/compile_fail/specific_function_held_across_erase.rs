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
        let mut module = context.create_module("m");
        let function = module.add_specific_function::<Function<Void, ()>>("f");
        let entry = module.value_to_erase(function);
        module.erase(&[entry]);
        let _ = function.as_function();
    });
}
