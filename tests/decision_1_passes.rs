//! Passes in Rust, linked into a compiler: pipelines, extension points, analyses, cloning and inlining (D1).

use std::cell::{Cell, RefCell};

use lyftar::module::Module;
use lyftar::passes::{
    Analyses, FunctionInPass, FunctionPass, ModulePass, OptimizationLevel, PassOptions, PreservedAnalyses,
};
use lyftar::values::CallBase;
use lyftar::{Builder, with_context};

const IR: &str = "
define i64 @sum(i64 %n) {
entry:
  br label %outer
outer:
  %i = phi i64 [ 0, %entry ], [ %i.next, %outer.latch ]
  %total = phi i64 [ 0, %entry ], [ %inner.total, %outer.latch ]
  br label %inner
inner:
  %j = phi i64 [ 0, %outer ], [ %j.next, %inner ]
  %inner.total = phi i64 [ %total, %outer ], [ %t, %inner ]
  %t = add i64 %inner.total, %j
  %j.next = add i64 %j, 1
  %inner.more = icmp ult i64 %j.next, %n
  br i1 %inner.more, label %inner, label %outer.latch
outer.latch:
  %i.next = add i64 %i, 1
  %outer.more = icmp ult i64 %i.next, %n
  br i1 %outer.more, label %outer, label %exit
exit:
  ret i64 %inner.total
}
define i64 @callee(i64 %x) {
  %y = add i64 %x, 1
  ret i64 %y
}
define i64 @caller(i64 %x) {
  %r = call i64 @callee(i64 %x)
  ret i64 %r
}
declare void @nothing()
";

struct LooksAtLoops<'report> {
    report: &'report RefCell<Vec<String>>,
}

impl ModulePass for LooksAtLoops<'_> {
    fn run<'context>(&mut self, module: &mut Module<'context>, analyses: &Analyses<'_>) -> PreservedAnalyses {
        let sum = module.get_function("sum").expect("the function");
        let block = |name: &str| sum.basic_blocks().find(|found| found.name() == name).expect("the block");
        let (entry, outer, inner, latch, exit) =
            (block("entry"), block("outer"), block("inner"), block("outer.latch"), block("exit"));
        let tree = analyses.dominator_tree(module, sum);
        assert_eq!(tree.root(), entry);
        assert!(tree.dominates(entry, exit));
        assert!(tree.dominates(outer, inner));
        assert!(!tree.dominates(inner, outer));
        assert!(tree.properly_dominates(outer, latch));
        assert!(!tree.properly_dominates(outer, outer));
        assert_eq!(tree.immediate_dominator(inner), Some(outer));
        assert_eq!(tree.immediate_dominator(entry), None);
        assert!(tree.is_reachable(exit));
        assert_eq!(tree.children(entry), [outer]);
        let first = entry.first_instruction().expect("an instruction");
        let last = exit.last_instruction().expect("an instruction");
        assert!(tree.value_dominates(first, last));
        assert!(!tree.value_dominates(last, first));

        let loops = analyses.loop_info(module, sum);
        let top = loops.top_level_loops();
        assert_eq!(top.len(), 1);
        let outer_loop = top[0];
        assert_eq!(outer_loop.header(), outer);
        assert_eq!(outer_loop.depth(), 1);
        assert_eq!(outer_loop.preheader(), Some(entry));
        assert_eq!(outer_loop.latch(), Some(latch));
        assert_eq!(outer_loop.parent(), None);
        assert_eq!(outer_loop.exit_basic_blocks(), [exit]);
        assert!(outer_loop.contains(inner));
        assert!(!outer_loop.contains(exit));
        let inner_loop = loops.loop_for(inner).expect("in a loop");
        assert_eq!(inner_loop.depth(), 2);
        assert_eq!(inner_loop.parent(), Some(outer_loop));
        assert_eq!(outer_loop.subloops(), [inner_loop]);
        assert_eq!(outer_loop.basic_blocks().len(), 3);
        assert_eq!(loops.loop_for(exit), None);
        self.report.borrow_mut().push("looked".to_string());
        PreservedAnalyses::All
    }
}

#[test]
fn decision_1_a_pass_can_ask_for_dominator_trees_and_loops() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let report = RefCell::new(Vec::new());
        module
            .run_passes_with("looks-at-loops", None, &PassOptions::default(), |builder| {
                let report = &report;
                builder.register_module_pass("looks-at-loops", move || LooksAtLoops { report });
            })
            .expect("the pipeline");
        assert_eq!(*report.borrow(), ["looked"]);
    });
}

/// Erases a block, then asks for the tree again: it must be a fresh one, with no trace of the block.
struct FreshAfterErase;

impl ModulePass for FreshAfterErase {
    fn run<'context>(&mut self, module: &mut Module<'context>, analyses: &Analyses<'_>) -> PreservedAnalyses {
        {
            let function = module.get_function("two_blocks").expect("the function");
            let tree = analyses.dominator_tree(module, function);
            assert_eq!(tree.children(function.first_basic_block().expect("a body")).len(), 1);
        }
        let entry = {
            let function = module.get_function("two_blocks").expect("the function");
            let second = function.basic_blocks().nth(1).expect("a second block");
            module.basic_block_to_erase(second)
        };
        let terminator = {
            let function = module.get_function("two_blocks").expect("the function");
            module.value_to_erase(function.first_basic_block().expect("a body").terminator().expect("a terminator"))
        };
        module.erase(&[entry, terminator]);
        {
            let function = module.get_function("two_blocks").expect("the function");
            let builder = Builder::at_end_of(function.first_basic_block().expect("a body"));
            builder.return_void();
            let tree = analyses.dominator_tree(module, function);
            assert_eq!(tree.children(function.first_basic_block().expect("a body")).len(), 0);
        }
        PreservedAnalyses::None
    }
}

#[test]
fn decision_11_analyses_are_fresh_after_a_delete() {
    with_context(|context| {
        let mut module = context
            .parse_ir_text("define void @two_blocks() {\nfirst:\n br label %second\nsecond:\n ret void\n}")
            .expect("good IR");
        module
            .run_passes_with("fresh", None, &PassOptions::default(), |builder| {
                builder.register_module_pass("fresh", || FreshAfterErase);
            })
            .expect("the pipeline");
        assert_eq!(module.verify(), Ok(()));
    });
}

struct InlinesAndClones;

impl ModulePass for InlinesAndClones {
    fn run<'context>(&mut self, module: &mut Module<'context>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
        let choice = {
            let callee = module.get_function("callee").expect("the function");
            let clone = callee.clone_function();
            assert_eq!(clone.instruction_count(), callee.instruction_count());
            clone.set_name("callee.clone");
            let caller = module.get_function("caller").expect("the function");
            let call = caller.function_instructions().find_map(|found| found.as_kind::<CallBase>()).expect("a call");
            module.call_to_inline(call)
        };
        module.inline_call(choice).expect("inlining");
        // A declaration can't be inlined, which is a real failure that comes back as an error.
        let refused = {
            let nothing = module.get_function("nothing").expect("the function");
            let callee = module.get_function("callee").expect("the function");
            let builder =
                Builder::before(callee.first_basic_block().expect("a body").first_instruction().expect("one"));
            let none: [lyftar::Value<'_, '_>; 0] = [];
            let call = builder.call(nothing, &none, "");
            module.call_to_inline(call)
        };
        assert!(module.inline_call(refused).is_err());
        PreservedAnalyses::None
    }
}

#[test]
fn decision_1_a_pass_can_clone_and_inline() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        module
            .run_passes_with("inlines", None, &PassOptions::default(), |builder| {
                builder.register_module_pass("inlines", || InlinesAndClones);
            })
            .expect("the pipeline");
        assert_eq!(module.verify(), Ok(()));
        let caller = module.get_function("caller").expect("the function");
        assert_eq!(caller.function_instructions().filter(|found| found.as_kind::<CallBase>().is_some()).count(), 0);
        assert!(module.get_function("callee.clone").is_some());
    });
}

struct Counts<'report> {
    runs: &'report Cell<u32>,
}

impl FunctionPass for Counts<'_> {
    fn run<'context>(
        &mut self,
        _function: &mut FunctionInPass<'context>,
        _analyses: &Analyses<'_>,
    ) -> PreservedAnalyses {
        self.runs.set(self.runs.get() + 1);
        PreservedAnalyses::All
    }
}

impl ModulePass for Counts<'_> {
    fn run<'context>(&mut self, _module: &mut Module<'context>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
        self.runs.set(self.runs.get() + 1);
        PreservedAnalyses::All
    }
}

#[test]
fn decision_1_each_extension_point_runs_a_rust_pass_in_a_default_pipeline() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let (peephole, scalar, vectorize_start, vectorize_end, start, early, optimizer_early, optimizer_last, graph) = (
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
            Cell::new(0),
        );
        let loop_late = Cell::new(0);
        let loop_end = Cell::new(0);
        let levels: RefCell<Vec<OptimizationLevel>> = RefCell::new(Vec::new());
        module
            .run_passes_with("default<O3>", None, &PassOptions::default(), |builder| {
                builder.at_peephole(|manager, level| {
                    levels.borrow_mut().push(level);
                    manager.add_function_pass(Counts { runs: &peephole });
                });
                builder.at_scalar_optimizer_late(|manager, _| manager.add_function_pass(Counts { runs: &scalar }));
                builder.at_vectorizer_start(|manager, _| manager.add_function_pass(Counts { runs: &vectorize_start }));
                builder.at_vectorizer_end(|manager, _| manager.add_function_pass(Counts { runs: &vectorize_end }));
                builder.at_pipeline_start(|manager, _| manager.add_module_pass(Counts { runs: &start }));
                builder
                    .at_pipeline_early_simplification(|manager, _, _| manager.add_module_pass(Counts { runs: &early }));
                builder
                    .at_optimizer_early(|manager, _, _| manager.add_function_pass(Counts { runs: &optimizer_early }));
                builder.at_optimizer_last(|manager, _, _| manager.add_module_pass(Counts { runs: &optimizer_last }));
                builder.at_call_graph_optimizer_late(|manager, _| manager.add_function_pass(Counts { runs: &graph }));
                builder.at_late_loop_optimizations(|manager, _| {
                    loop_late.set(loop_late.get() + 1);
                    manager.add_pipeline("loop-instsimplify").expect("LLVM's own pass");
                });
                builder.at_loop_optimizer_end(|manager, _| {
                    loop_end.set(loop_end.get() + 1);
                    manager.add_pipeline("loop-instsimplify").expect("LLVM's own pass");
                });
            })
            .expect("the pipeline");
        assert_eq!(module.verify(), Ok(()));
        for (name, count) in [
            ("peephole", &peephole),
            ("scalar", &scalar),
            ("vectorize_start", &vectorize_start),
            ("vectorize_end", &vectorize_end),
            ("start", &start),
            ("early", &early),
            ("optimizer_early", &optimizer_early),
            ("optimizer_last", &optimizer_last),
            ("graph", &graph),
            ("loop_late", &loop_late),
            ("loop_end", &loop_end),
        ] {
            assert!(count.get() > 0, "{name} didn't run");
        }
        assert!(levels.borrow().contains(&OptimizationLevel::Level3));
    });
}

#[test]
fn decision_1_link_time_extension_points_run() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let (early, last) = (Cell::new(0), Cell::new(0));
        module
            .run_passes_with("lto<O2>", None, &PassOptions::default(), |builder| {
                builder.at_full_link_time_optimization_early(|manager, _| {
                    manager.add_module_pass(Counts { runs: &early })
                });
                builder
                    .at_full_link_time_optimization_last(|manager, _| manager.add_module_pass(Counts { runs: &last }));
            })
            .expect("the pipeline");
        assert!(early.get() > 0 && last.get() > 0);
    });
}

#[test]
fn decision_3_a_bad_pipeline_comes_back_as_an_error() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let result = module.run_passes_with("no-such-pass", None, &PassOptions::default(), |_| {});
        assert!(result.is_err_and(|error| error.message().contains("no-such-pass")));
    });
}

#[test]
fn decision_1_pass_options_reach_the_pipeline() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let options = PassOptions {
            verify_each: true,
            loop_vectorization: false,
            loop_unrolling: false,
            superword_level_parallelism_vectorization: false,
            merge_functions: true,
            inliner_threshold: Some(10),
            alias_analysis_pipeline: Some("basic-aa".to_string()),
            ..PassOptions::default()
        };
        module.run_passes_with("default<O2>", None, &options, |_| {}).expect("the pipeline");
        assert_eq!(module.verify(), Ok(()));
        module.run_passes("default<O1>", None, &options).expect("the pipeline");
    });
}

// LLVM ends the process over these, or frees an invalid pointer, unless they are caught first.
#[test]
fn decision_38_what_llvm_would_end_the_process_over_comes_back_as_an_error() {
    with_context(|context| {
        for (pipeline, wanted) in [
            ("default<Os>", "the optimization level \"Os\" is no longer supported"),
            ("default<Oz>", "the optimization level \"Oz\" is no longer supported"),
            ("lto<Os>", "the optimization level \"Os\" is no longer supported"),
            ("thinlto<Oz>", "the optimization level \"Oz\" is no longer supported"),
            ("lto-pre-link<Os>", "the optimization level \"Os\" is no longer supported"),
            ("thinlto-pre-link<Oz>", "the optimization level \"Oz\" is no longer supported"),
            ("function(sroa),default<Os>", "the optimization level \"Os\" is no longer supported"),
            ("function-import", "needs LLVM's -summary-file option"),
            ("globaldce,function-import", "needs LLVM's -summary-file option"),
            ("function(free-machine-function)", "needs a machine function analysis"),
        ] {
            let mut module = context.parse_ir_text(IR).expect("the test's IR");
            let error = module.run_passes(pipeline, None, &PassOptions::default()).expect_err(pipeline);
            assert!(error.message().contains(wanted), "{pipeline}: {}", error.message());
            let error = module.run_passes_with(pipeline, None, &PassOptions::default(), |_| {}).expect_err(pipeline);
            assert!(error.message().contains(wanted), "{pipeline}: {}", error.message());
            assert_eq!(module.verify(), Ok(()));
        }
        // A pass of someone's own can take a parameter of that spelling, and a similar name is a different pass.
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let result = module.run_passes_with("own-pass<Os>", None, &PassOptions::default(), |_| {});
        assert!(result.is_err_and(|error| error.message().contains("own-pass")));
        let result = module.run_passes("function-import-not", None, &PassOptions::default());
        assert!(result.is_err_and(|error| error.message().contains("function-import-not")));
        assert!(module.run_passes("default<O2>", None, &PassOptions::default()).is_ok());
    });
}

#[test]
fn decision_38_what_an_extension_point_adds_is_checked_too() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let found = RefCell::new(None);
        module
            .run_passes_with("default<O1>", None, &PassOptions::default(), |builder| {
                builder.at_pipeline_start(|manager, _| {
                    *found.borrow_mut() = Some(manager.add_pipeline("default<Oz>").expect_err("Oz"));
                });
            })
            .expect("the pipeline");
        let found = found.into_inner().expect("the extension point ran");
        assert!(found.message().contains("\"Oz\" is no longer supported"), "{}", found.message());
    });
}

fn with_unterminated_function(context: lyftar::Context<'_>, body: impl FnOnce(&mut Module<'_>)) {
    let mut module = context.create_module("invalid");
    {
        let function = module.add_function("f", context.function_type(context.void_type(), &[], false));
        let _ = function.append_basic_block("entry");
    }
    body(&mut module);
}

// LLVM segfaults on each of these.
#[test]
fn decision_38_an_invalid_module_is_an_error_rather_than_a_crash() {
    with_context(|context| {
        let wanted = "fails LLVM's verifier: Basic Block in function 'f' does not have terminator";
        with_unterminated_function(context, |module| {
            let error = module.run_passes("default<O2>", None, &PassOptions::default()).expect_err("invalid");
            assert!(
                error.message().contains(wanted) && error.message().starts_with("run_passes:"),
                "{}",
                error.message()
            );
            let error =
                module.run_passes_with("default<O2>", None, &PassOptions::default(), |_| {}).expect_err("invalid");
            assert!(
                error.message().contains(wanted) && error.message().starts_with("run_passes_with:"),
                "{}",
                error.message()
            );
            let error = module
                .run_passes_on_function("f", "function(sroa)", None, &PassOptions::default())
                .expect_err("invalid");
            assert!(
                error.message().contains(wanted) && error.message().starts_with("run_passes_on_function:"),
                "{}",
                error.message()
            );
        });
        let machine = lyftar::target::TargetMachine::for_host().expect("this machine");
        with_unterminated_function(context, |module| {
            let file_type = lyftar::target::CodeGenerationFileType::Object;
            let error = machine.emit_to_memory_buffer(module, file_type).err().expect("invalid");
            assert!(
                error.message().contains(wanted) && error.message().starts_with("emit_to_memory_buffer:"),
                "{}",
                error.message()
            );
            let path = std::env::temp_dir().join(format!("lyftar-invalid-{}.o", std::process::id()));
            let error = machine.emit_to_file(module, &path, file_type).expect_err("invalid");
            assert!(
                error.message().contains(wanted) && error.message().starts_with("emit_to_file:"),
                "{}",
                error.message()
            );
            assert!(!path.exists());
        });
    });
}

// A `llvm.dbg.declare` taking a data array made `strip-debug-declare` free plain data, which D37 says survives.
#[test]
fn decision_38_a_malformed_debug_declare_cant_free_plain_data() {
    with_context(|context| {
        let i32_type = context.get_type::<lyftar::Integer<lyftar::Bits<32>>>();
        let data = i32_type.constant_data_array(&[1, 0, 0, 0, 2, 0, 0, 0]);
        let mut module = context.create_module("invalid");
        {
            let declare = module.add_function(
                "llvm.dbg.declare",
                context.function_type(context.void_type(), &[data.get_type()], false),
            );
            let user = module.add_function("user", context.function_type(context.void_type(), &[], false));
            let builder = Builder::at_end_of(user.append_basic_block("entry"));
            builder.call(declare, &[data], "");
            builder.return_void();
        }
        assert!(module.run_passes("strip-debug-declare", None, &PassOptions::default()).is_err());
        assert_eq!(data.print_to_string(), "[2 x i32] [i32 1, i32 2]");
    });
}
