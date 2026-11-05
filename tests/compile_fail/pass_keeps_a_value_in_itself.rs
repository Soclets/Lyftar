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

struct P<'stored> {
    kept: Option<Value<'stored, 'stored, Function>>,
}
impl<'stored> ModulePass for P<'stored> {
    fn run<'context>(&mut self, module: &mut Module<'context>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
        let function = module.get_function("f");
        self.kept = function;
        PreservedAnalyses::All
    }
}
fn main() {
    let _ = P { kept: None };
}
