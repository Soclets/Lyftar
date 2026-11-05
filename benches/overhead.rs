//! What Lyftar adds over raw LLVM calls (D24). Each row builds the same instructions both ways, in the same conditions.
//!
//! `cargo bench --bench overhead` prints the table beside the saved one in `benches/overhead_saved.txt`, so a change that
//! slows a build call shows up. `cargo bench --bench overhead -- --save` saves this run.

use std::fmt::Write;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

use lyftar::enums::IntegerPredicate;
use lyftar::passes::{Analyses, FunctionInPass, FunctionPass, PassOptions, PreservedAnalyses};
use lyftar::raw;
use lyftar::{
    BasicBlock, Bits, Builder, Context, Dynamic, Function, Integer, Module, Pointer, Type, Value, with_context,
};

type I64 = Integer<Bits<64>>;

/// How many calls one batch builds. A batch's function is thrown away after it, so LLVM's cost stays flat.
const CALLS_IN_A_BATCH: u32 = 2000;
const BATCHES: u32 = 60;

const EMPTY: *const std::ffi::c_char = c"".as_ptr();

struct Scene<'context, 'module> {
    context: Context<'context>,
    builder: &'module Builder<'context, 'module>,
    raw_builder: raw::LLVMBuilderRef,
    left: Value<'context, 'module, I64>,
    right: Value<'context, 'module, I64>,
    pointer: Value<'context, 'module, Pointer>,
    callee: Value<'context, 'module, Function>,
    specific_callee: Value<'context, 'module, Function<I64, (I64, I64)>>,
    function: raw::LLVMValueRef,
}

type Step = for<'context, 'module> fn(&Scene<'context, 'module>);

// SAFETY for every raw call below: the scene's values and builder are live, and the operands are what LLVM requires.
const ROWS: [(&str, Step, Step); 9] = [
    (
        "integer_add, specific",
        |scene| unsafe {
            black_box(raw::LLVMBuildAdd(scene.raw_builder, scene.left.as_raw(), scene.right.as_raw(), EMPTY));
        },
        |scene| {
            black_box(scene.builder.integer_add(scene.left, scene.right, ""));
        },
    ),
    (
        "integer_add, dynamic",
        |scene| unsafe {
            black_box(raw::LLVMBuildAdd(scene.raw_builder, scene.left.as_raw(), scene.right.as_raw(), EMPTY));
        },
        |scene| {
            let (left, right) = (scene.left.to_dynamic(), scene.right.to_dynamic());
            let left: Value<'_, '_, Integer<Dynamic>> = left.checked_cast();
            let right: Value<'_, '_, Integer<Dynamic>> = right.checked_cast();
            black_box(scene.builder.integer_add(left, right, ""));
        },
    ),
    (
        "integer_compare, specific",
        |scene| unsafe {
            black_box(raw::LLVMBuildICmp(
                scene.raw_builder,
                raw::LLVMIntPredicate::LLVMIntEQ,
                scene.left.as_raw(),
                scene.right.as_raw(),
                EMPTY,
            ));
        },
        |scene| {
            black_box(scene.builder.integer_compare(IntegerPredicate::Equal, scene.left, scene.right, ""));
        },
    ),
    (
        "load, specific",
        |scene| unsafe {
            let loaded = scene.context.get_type::<I64>().as_raw();
            black_box(raw::LLVMBuildLoad2(scene.raw_builder, loaded, scene.pointer.as_raw(), EMPTY));
        },
        |scene| {
            black_box(scene.builder.load(scene.context.get_type::<I64>(), scene.pointer, ""));
        },
    ),
    (
        "store, specific",
        |scene| unsafe {
            black_box(raw::LLVMBuildStore(scene.raw_builder, scene.left.as_raw(), scene.pointer.as_raw()));
        },
        |scene| {
            black_box(scene.builder.store(scene.left, scene.pointer));
        },
    ),
    (
        "get_element_pointer",
        |scene| unsafe {
            let source: Type<'_> = scene.context.get_type::<I64>().to_dynamic();
            let mut indices = [scene.left.as_raw()];
            black_box(raw::LLVMBuildGEP2(
                scene.raw_builder,
                source.as_raw(),
                scene.pointer.as_raw(),
                indices.as_mut_ptr(),
                1,
                EMPTY,
            ));
        },
        |scene| {
            let source: Type<'_> = scene.context.get_type::<I64>().to_dynamic();
            black_box(scene.builder.get_element_pointer(source, scene.pointer, &[scene.left], ""));
        },
    ),
    (
        "call, two arguments",
        |scene| unsafe {
            let function_type = raw::LLVMGlobalGetValueType(scene.callee.as_raw());
            let mut arguments = [scene.left.as_raw(), scene.right.as_raw()];
            black_box(raw::LLVMBuildCall2(
                scene.raw_builder,
                function_type,
                scene.callee.as_raw(),
                arguments.as_mut_ptr(),
                2,
                EMPTY,
            ));
        },
        |scene| {
            black_box(scene.builder.call(scene.callee, &[scene.left, scene.right], ""));
        },
    ),
    (
        "call, two arguments, specific",
        |scene| unsafe {
            let function_type = raw::LLVMGlobalGetValueType(scene.specific_callee.as_raw());
            let mut arguments = [scene.left.as_raw(), scene.right.as_raw()];
            black_box(raw::LLVMBuildCall2(
                scene.raw_builder,
                function_type,
                scene.specific_callee.as_raw(),
                arguments.as_mut_ptr(),
                2,
                EMPTY,
            ));
        },
        |scene| {
            black_box(scene.builder.call(scene.specific_callee, (scene.left, scene.right), ""));
        },
    ),
    (
        "branch into a new block",
        |scene| unsafe {
            let next = raw::LLVMAppendBasicBlockInContext(scene.context.as_raw(), scene.function, EMPTY);
            black_box(raw::LLVMBuildBr(scene.raw_builder, next));
            raw::LLVMPositionBuilderAtEnd(scene.raw_builder, next);
        },
        |scene| unsafe {
            // The block is made the same way in both, so that this row measures the branch and the move.
            let next =
                BasicBlock::from_raw(raw::LLVMAppendBasicBlockInContext(scene.context.as_raw(), scene.function, EMPTY));
            black_box(scene.builder.branch(next));
            scene.builder.position_at_end(next);
        },
    ),
];

/// The best batch, in nanoseconds per call: the least disturbed by anything else on this machine.
fn best_nanoseconds_per_call(step: Step) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..BATCHES {
        let elapsed = with_context(|context| {
            let module: Module<'_> = context.create_module("overhead");
            let i64_type = context.get_type::<I64>().to_dynamic();
            let pointer_type = context.pointer_type(0).to_dynamic();
            let callee_type = context.function_type(i64_type, &[i64_type, i64_type], false);
            let callee = module.add_function("callee", callee_type);
            let specific_callee = module.add_specific_function::<Function<I64, (I64, I64)>>("specific_callee");
            let function_type = context.function_type(i64_type, &[i64_type, i64_type, pointer_type], false);
            let function = module.add_function("function", function_type);
            let entry = function.append_basic_block("entry");
            let builder = Builder::at_end_of(entry);
            let left = function.parameter(0).checked_cast::<I64>();
            let right = function.parameter(1).checked_cast::<I64>();
            let pointer = function.parameter(2).checked_cast::<Pointer>();
            // SAFETY: only read here, and the builder outlives the scene.
            let raw_builder = unsafe { builder.as_raw() };
            let scene = Scene {
                context,
                builder: &builder,
                raw_builder,
                left,
                right,
                pointer,
                callee,
                specific_callee,
                // SAFETY: only read here, and the function outlives the scene.
                function: unsafe { function.as_raw() },
            };
            let start = Instant::now();
            for _ in 0..CALLS_IN_A_BATCH {
                step(&scene);
            }
            start.elapsed()
        });
        best = best.min(elapsed.as_secs_f64() * 1e9 / f64::from(CALLS_IN_A_BATCH));
    }
    best
}

/// A pass that does nothing, so what it costs is what Lyftar adds around a Rust pass: the verifier (D40) and the hash that
/// checks its claim of changing nothing (D3).
struct DoesNothing;

impl FunctionPass for DoesNothing {
    fn run<'context>(
        &mut self,
        _function: &mut FunctionInPass<'context>,
        _analyses: &Analyses<'_>,
    ) -> PreservedAnalyses {
        PreservedAnalyses::All
    }
}

const FUNCTIONS: u32 = 200;
const ADDS_IN_A_FUNCTION: u32 = 200;

/// The best run over a module of many functions, in nanoseconds per function a pipeline visits.
fn best_nanoseconds_per_function(pipeline: &str, rust_pass: bool) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..20 {
        let elapsed = with_context(|context| {
            let mut module: Module<'_> = context.create_module("passes");
            let i64_type = context.get_type::<I64>().to_dynamic();
            let function_type = context.function_type(i64_type, &[i64_type], false);
            for index in 0..FUNCTIONS {
                let function = module.add_function(&format!("f{index}"), function_type);
                let builder = Builder::at_end_of(function.append_basic_block("entry"));
                let parameter = function.parameter(0).checked_cast::<I64>();
                let mut total = parameter;
                for _ in 0..ADDS_IN_A_FUNCTION {
                    total = builder.integer_add(total, parameter, "");
                }
                builder.return_value(total);
            }
            let options = PassOptions::default();
            let start = Instant::now();
            if rust_pass {
                module
                    .run_passes_with(pipeline, None, &options, |builder| {
                        builder.register_function_pass("does-nothing", || DoesNothing);
                    })
                    .expect("the pipeline");
            } else {
                module.run_passes(pipeline, None, &options).expect("the pipeline");
            }
            start.elapsed()
        });
        best = best.min(elapsed.as_secs_f64() * 1e9 / f64::from(FUNCTIONS));
    }
    best
}

fn main() {
    let saved_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/overhead_saved.txt");
    let saved = std::fs::read_to_string(&saved_path).unwrap_or_default();
    let mut table = String::new();
    println!("{:<30}{:>12}{:>12}{:>12}{:>16}", "nanoseconds per call", "raw", "Lyftar", "added", "added, saved");
    for (name, raw_step, lyftar_step) in ROWS {
        let raw_time = best_nanoseconds_per_call(raw_step);
        let lyftar_time = best_nanoseconds_per_call(lyftar_step);
        let added = lyftar_time - raw_time;
        let before = saved
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('\t'))
            .map_or("none".to_string(), str::to_string);
        println!("{name:<30}{raw_time:>12.1}{lyftar_time:>12.1}{added:>12.1}{before:>16}");
        let _ = writeln!(table, "{name}\t{added:.1}");
    }
    // trace: D40, D24
    let name = "Rust function pass, 200 adds";
    let raw_time = best_nanoseconds_per_function("function(no-op-function)", false);
    let lyftar_time = best_nanoseconds_per_function("function(does-nothing)", true);
    let added = lyftar_time - raw_time;
    let before = saved
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix('\t'))
        .map_or("none".to_string(), str::to_string);
    println!(
        "{:<30}{:>12}{:>12}{:>12}{:>16}",
        "nanoseconds per function", "LLVM no-op", "Rust no-op", "added", "added, saved"
    );
    println!("{name:<30}{raw_time:>12.1}{lyftar_time:>12.1}{added:>12.1}{before:>16}");
    let _ = writeln!(table, "{name}\t{added:.1}");
    if std::env::args().any(|argument| argument == "--save") {
        std::fs::write(&saved_path, table).expect("saving the table");
    }
}
