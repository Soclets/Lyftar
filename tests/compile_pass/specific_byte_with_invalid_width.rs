#![allow(unused_imports)]

use lyftar::enums::*;
use lyftar::passes::*;
use lyftar::target::*;
use lyftar::values::*;
use lyftar::*;

type B8 = Byte<Bits<8>>;
type B16 = Byte<Bits<16>>;
type I8 = Integer<Bits<8>>;

fn main() {
    with_context(|context| {
        let _ = context.get_type::<Byte<Bits<1>>>();
    });
}
