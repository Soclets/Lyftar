#![allow(unused_imports)]

use lyftar::module::Module;
use lyftar::passes::*;

struct Replaces;

impl ModulePass for Replaces {
    fn run(&mut self, module: &mut Module<'_>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
        // A pass can't be handed another module's insides: what it makes is a handle, which isn't a `Module`.
        *module = module.context().create_module("replacement");
        PreservedAnalyses::None
    }
}

fn main() {}
