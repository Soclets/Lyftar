#![allow(unused_imports)]

use lyftar::enums::*;
use lyftar::passes::*;
use lyftar::target::*;
use lyftar::values::*;
use lyftar::*;

type I32 = Integer<Bits<32>>;
type I64 = Integer<Bits<64>>;
type F32 = Float<Ieee754Binary<Bits<32>>>;
type F64 = Float<Ieee754Binary<Bits<64>>>;

struct P;
impl FunctionPass for P {
    fn run<'context>(&mut self, function: &mut FunctionInPass<'context>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
        let value = function.function();
        let block = value.first_basic_block().unwrap();
        let first = block.first_instruction().unwrap();
        let chosen = [function.value_to_erase(first)];
        let _ = first.print_to_string();
        function.erase(&chosen);
        PreservedAnalyses::None
    }
}
fn main() {
    let _ = P;
}
