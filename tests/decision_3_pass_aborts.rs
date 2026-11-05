//! Inside a Rust pass, a panic aborts the compiler with its message, since the IR may be half rewritten, and no panic crosses
//! into LLVM's C++ (D3). Each case runs in a child process, as it aborts.

use lyftar::module::{Module, ValueToErase};
use lyftar::passes::{Analyses, FunctionInPass, FunctionPass, ModulePass, PassOptions, PreservedAnalyses};
use lyftar::{Bits, Builder, Integer, with_context};

const IR: &str = "
define i64 @other(i64 %x) {
  ret i64 %x
}
define i64 @f(i64 %x) {
entry:
  %y = add i64 %x, 1
  %z = call i64 @other(i64 %y)
  ret i64 %z
}
declare void @nobody()
";

// LLVM can call a registered pass's maker more than once while it reads the text, so these take a maker.
fn run_module_pass<P: ModulePass + 'static>(make: fn() -> P) {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        module
            .run_passes_with("test-pass", None, &PassOptions::default(), |builder| {
                builder.register_module_pass("test-pass", make);
            })
            .expect("the pipeline");
    });
}

fn run_function_pass<P: FunctionPass + 'static>(make: fn() -> P) {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        module
            .run_passes_with("function(test-pass)", None, &PassOptions::default(), |builder| {
                builder.register_function_pass("test-pass", move || Once { pass: make(), done: false });
            })
            .expect("the pipeline");
    });
}

// A function pass is made once by name and runs on each function, so this runs the case on `f` only.
struct Once<P> {
    pass: P,
    done: bool,
}

impl<P: FunctionPass> FunctionPass for Once<P> {
    fn run<'context>(&mut self, function: &mut FunctionInPass<'context>, analyses: &Analyses<'_>) -> PreservedAnalyses {
        if function.function().name() != "f" || self.done {
            return PreservedAnalyses::All;
        }
        self.done = true;
        self.pass.run(function, analyses)
    }
}

struct PanicsInPass;
impl FunctionPass for PanicsInPass {
    #[allow(clippy::panic)]
    fn run<'context>(
        &mut self,
        _function: &mut FunctionInPass<'context>,
        _analyses: &Analyses<'_>,
    ) -> PreservedAnalyses {
        panic!("the pass found a bug");
    }
}

struct Lies;
impl FunctionPass for Lies {
    fn run<'context>(
        &mut self,
        function: &mut FunctionInPass<'context>,
        _analyses: &Analyses<'_>,
    ) -> PreservedAnalyses {
        let entry = function.function().first_block().expect("a body");
        let first = entry.first_instruction().expect("an instruction");
        let builder = Builder::before(first);
        let i64_type = function.module().context().get_type::<Integer<Bits<64>>>();
        let _ = builder.stack_allocate(i64_type, "added");
        PreservedAnalyses::All
    }
}

struct LiesInModulePass;
impl ModulePass for LiesInModulePass {
    fn run<'context>(&mut self, module: &mut Module<'context>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
        let context = module.context();
        let _ = module.add_function("added", context.function_type(context.void_type(), &[], false));
        PreservedAnalyses::All
    }
}

struct ErasesElsewhereByValue;
impl FunctionPass for ErasesElsewhereByValue {
    fn run<'context>(
        &mut self,
        function: &mut FunctionInPass<'context>,
        _analyses: &Analyses<'_>,
    ) -> PreservedAnalyses {
        let other = function.module().get_function("other").expect("the other function");
        let first = other.first_block().expect("a body").first_instruction().expect("an instruction");
        let _ = function.value_to_erase(first);
        PreservedAnalyses::None
    }
}

struct ErasesElsewhereByBlock;
impl FunctionPass for ErasesElsewhereByBlock {
    fn run<'context>(
        &mut self,
        function: &mut FunctionInPass<'context>,
        _analyses: &Analyses<'_>,
    ) -> PreservedAnalyses {
        let other = function.module().get_function("other").expect("the other function");
        let _ = function.block_to_erase(other.first_block().expect("a body"));
        PreservedAnalyses::None
    }
}

struct ErasesElsewhereThroughTheModule;
impl FunctionPass for ErasesElsewhereThroughTheModule {
    fn run<'context>(
        &mut self,
        function: &mut FunctionInPass<'context>,
        _analyses: &Analyses<'_>,
    ) -> PreservedAnalyses {
        let entry: ValueToErase = {
            let other = function.module().get_function("nobody").expect("the other function");
            function.module().value_to_erase(other)
        };
        function.erase(&[entry]);
        PreservedAnalyses::None
    }
}

struct InlinesElsewhere;
impl FunctionPass for InlinesElsewhere {
    fn run<'context>(
        &mut self,
        function: &mut FunctionInPass<'context>,
        _analyses: &Analyses<'_>,
    ) -> PreservedAnalyses {
        // A call in another function of this module.
        let callee = function.module().get_function("other").expect("the other function");
        let builder = Builder::at_end_of(callee.append_block("extra"));
        let parameter = callee.parameter(0);
        let in_other = builder.call(callee, &[parameter], "");
        let _ = function.call_to_inline(in_other);
        PreservedAnalyses::None
    }
}

struct AnalysesForAnotherModule;
impl ModulePass for AnalysesForAnotherModule {
    fn run<'context>(&mut self, module: &mut Module<'context>, analyses: &Analyses<'_>) -> PreservedAnalyses {
        let other = module.clone_module();
        let function = other.get_function("f").expect("the function");
        let _ = analyses.dominator_tree(&other, function);
        PreservedAnalyses::None
    }
}

struct AnalysesOfADeclaration;
impl ModulePass for AnalysesOfADeclaration {
    fn run<'context>(&mut self, module: &mut Module<'context>, analyses: &Analyses<'_>) -> PreservedAnalyses {
        let function = module.get_function("nobody").expect("the function");
        let _ = analyses.loop_info(module, function);
        PreservedAnalyses::None
    }
}

struct BlockFromAnotherFunction;
impl ModulePass for BlockFromAnotherFunction {
    fn run<'context>(&mut self, module: &mut Module<'context>, analyses: &Analyses<'_>) -> PreservedAnalyses {
        let function = module.get_function("f").expect("the function");
        let other = module.get_function("other").expect("the function");
        let tree = analyses.dominator_tree(module, function);
        let _ = tree.is_reachable(other.first_block().expect("a body"));
        PreservedAnalyses::None
    }
}

struct LoopForAnotherFunction;
impl ModulePass for LoopForAnotherFunction {
    fn run<'context>(&mut self, module: &mut Module<'context>, analyses: &Analyses<'_>) -> PreservedAnalyses {
        let function = module.get_function("f").expect("the function");
        let other = module.get_function("other").expect("the function");
        let loops = analyses.loop_info(module, function);
        let _ = loops.loop_for(other.first_block().expect("a body"));
        PreservedAnalyses::None
    }
}

struct ReplacesItsModule;
impl ModulePass for ReplacesItsModule {
    fn run<'context>(&mut self, module: &mut Module<'context>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
        *module = module.context().create_module("replacement");
        PreservedAnalyses::None
    }
}

/// Runs case `name` in this process.
fn run_case(name: &str) {
    match name {
        "panics" => run_function_pass(|| PanicsInPass),
        "lies" => run_function_pass(|| Lies),
        "lies_in_module_pass" => run_module_pass(|| LiesInModulePass),
        "erases_elsewhere_by_value" => run_function_pass(|| ErasesElsewhereByValue),
        "erases_elsewhere_by_block" => run_function_pass(|| ErasesElsewhereByBlock),
        "erases_elsewhere_through_the_module" => run_function_pass(|| ErasesElsewhereThroughTheModule),
        "inlines_elsewhere" => run_function_pass(|| InlinesElsewhere),
        "analyses_for_another_module" => run_module_pass(|| AnalysesForAnotherModule),
        "analyses_of_a_declaration" => run_module_pass(|| AnalysesOfADeclaration),
        "block_from_another_function" => run_module_pass(|| BlockFromAnotherFunction),
        "loop_for_another_function" => run_module_pass(|| LoopForAnotherFunction),
        "replaces_its_module" => run_module_pass(|| ReplacesItsModule),
        _ => {}
    }
}

/// The child process's way in: `cargo test` runs this test too, where the variable isn't set and it does nothing.
#[test]
fn child_process_entry() {
    if let Ok(name) = std::env::var("LYFTAR_ABORT_CASE") {
        run_case(&name);
    }
}

fn aborts_with(name: &str, expected: &str) {
    let output = std::process::Command::new(std::env::current_exe().expect("this test's path"))
        .args(["--exact", "child_process_entry", "--nocapture"])
        .env("LYFTAR_ABORT_CASE", name)
        .output()
        .expect("running the child");
    let printed = String::from_utf8_lossy(&output.stderr);
    assert!(printed.contains(expected), "{name}: wanted {expected:?} in:\n{printed}");
    // A panic crossing LLVM's C++ would show up as something other than this: SIGABRT from `abort`.
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(output.status.signal(), Some(6), "{name} should abort");
}

#[test]
fn decision_3_a_panic_in_a_pass_aborts_with_its_message() {
    aborts_with("panics", "the pass found a bug");
}

#[test]
fn decision_3_a_function_pass_that_changes_the_ir_and_claims_it_didnt_stops() {
    aborts_with("lies", "said it changed nothing, but it changed the IR");
}

#[test]
fn decision_3_a_module_pass_that_changes_the_ir_and_claims_it_didnt_stops() {
    aborts_with("lies_in_module_pass", "said it changed nothing, but it changed the IR");
}

#[test]
fn decision_11_a_function_pass_cant_choose_an_instruction_elsewhere() {
    aborts_with(
        "erases_elsewhere_by_value",
        "value_to_erase: a function pass can only erase from its own function, and",
    );
}

#[test]
fn decision_11_a_function_pass_cant_choose_a_block_elsewhere() {
    aborts_with(
        "erases_elsewhere_by_block",
        "block_to_erase: a function pass can only erase from its own function, and",
    );
}

#[test]
fn decision_11_a_function_pass_cant_erase_elsewhere_through_its_module() {
    aborts_with("erases_elsewhere_through_the_module", "erase: a function pass can only change its own function");
}

#[test]
fn decision_11_a_function_pass_cant_inline_elsewhere() {
    aborts_with("inlines_elsewhere", "call_to_inline: a function pass can only inline into its own function, and");
}

#[test]
fn decision_11_analyses_of_another_module_stop() {
    aborts_with("analyses_for_another_module", "dominator_tree: this pass's analyses are for another module");
}

#[test]
fn decision_3_analyses_of_a_declaration_stop() {
    aborts_with("analyses_of_a_declaration", "nobody has no body");
}

#[test]
fn decision_3_a_block_from_another_function_stops_a_dominator_tree() {
    aborts_with("block_from_another_function", "isn't in this tree's function");
}

#[test]
fn decision_3_a_block_from_another_function_stops_loop_info() {
    aborts_with("loop_for_another_function", "isn't in this function");
}

#[test]
fn decision_11_a_pass_that_replaces_its_module_stops() {
    aborts_with("replaces_its_module", "a module pass replaced its module with another");
}
