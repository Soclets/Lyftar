#![allow(unused_imports)]

use lyftar::enums::*;
use lyftar::passes::*;
use lyftar::target::*;
use lyftar::values::*;
use lyftar::*;

type Space5 = Pointer<AddressSpace<5>>;

fn main() {
    with_context(|context| {
        let _ = context.get_type::<Pointer<AddressSpace<16777216>>>();
    });
}
