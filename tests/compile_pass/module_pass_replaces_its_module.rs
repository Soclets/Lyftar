#![allow(unused_imports)]

use lyftar::module::Module;
use lyftar::passes::*;

struct Replaces;

impl ModulePass for Replaces {
    fn run(&mut self, module: &mut Module<'_>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
        let _replacement = module.context().create_module("replacement");
        PreservedAnalyses::None
    }
}

fn main() {}
