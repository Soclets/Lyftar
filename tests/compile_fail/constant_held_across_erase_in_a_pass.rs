#![allow(unused_imports)]

use lyftar::module::Module;
use lyftar::passes::*;
use lyftar::*;

struct Holds;

impl ModulePass for Holds {
    fn run(&mut self, module: &mut Module<'_>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
        let context = module.context();
        let one = context.get_type::<Integer<Bits<32>>>().constant_integer(1, false);
        let held = module.context().constant_struct(&[one, one], false);
        let global = module.get_global("g").unwrap();
        let entry = module.value_to_erase(global);
        module.erase(&[entry]);
        let _ = held.print_to_string();
        PreservedAnalyses::None
    }
}

fn main() {}
