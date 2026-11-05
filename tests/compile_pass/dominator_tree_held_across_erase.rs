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
impl ModulePass for P {
    fn run<'context>(&mut self, module: &mut Module<'context>, analyses: &Analyses<'_>) -> PreservedAnalyses {
        let function = module.get_function("f").unwrap();
        let tree = analyses.dominator_tree(module, function);
        let root = tree.root();
        let chosen = [module.basic_block_to_erase(root)];
        let _ = tree.root();
        module.erase(&chosen);
        PreservedAnalyses::None
    }
}
fn main() {
    let _ = P;
}
