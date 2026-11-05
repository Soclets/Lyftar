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
        let module = context.create_module("m");
        let module = context.module(&module);
        let _ = module.add_specific_function::<Function<Void, (Void,)>>("f");
    });
}
