//! kayo's passes in Rust against kayo's own pass tests (`CyborgPL/kayo/llvm-passes/test`), linked into a compiler, and through
//! `opt -load-pass-plugin`.

#[path = "../examples/kayo/heap_to_stack.rs"]
mod heap_to_stack;
#[path = "../examples/kayo/recursive_inline.rs"]
mod recursive_inline;

use std::cell::Cell;
use std::process::Command;

use heap_to_stack::HeapToStack;
use lyftar::module::Module;
use lyftar::passes::PassOptions;
use lyftar::values::{CallBase, Function};
use lyftar::{Value, with_context};
use recursive_inline::RecursiveInline;

// The same IR as kayo's `recursive_inline_test.cpp`.
const RECURSIVE_INLINE_IR: &str = r"
define i64 @fib(i64 %n) {
entry:
  %small = icmp ult i64 %n, 2
  br i1 %small, label %done, label %more
more:
  %a = sub i64 %n, 1
  %fa = call i64 @fib(i64 %a)
  %b = sub i64 %n, 2
  %fb = call i64 @fib(i64 %b)
  %s = add i64 %fa, %fb
  ret i64 %s
done:
  ret i64 %n
}

declare void @say(i64)

define void @countdown(i64 %n) {
entry:
  %z = icmp eq i64 %n, 0
  br i1 %z, label %done, label %more
more:
  call void @say(i64 %n)
  %m = sub i64 %n, 1
  call void @countdown(i64 %m)
  call void @say(i64 %n)
  ret void
done:
  ret void
}

define i64 @kept(i64 %n) noinline {
  %z = icmp eq i64 %n, 0
  br i1 %z, label %done, label %more
more:
  %m = sub i64 %n, 1
  %r = call i64 @kept(i64 %m)
  ret i64 %r
done:
  ret i64 0
}

define i64 @even(i64 %n) {
  %z = icmp eq i64 %n, 0
  br i1 %z, label %done, label %more
more:
  %m = sub i64 %n, 1
  %r = call i64 @odd(i64 %m)
  ret i64 %r
done:
  ret i64 1
}

define i64 @odd(i64 %n) {
  %z = icmp eq i64 %n, 0
  br i1 %z, label %done, label %more
more:
  %m = sub i64 %n, 1
  %r = call i64 @even(i64 %m)
  ret i64 %r
done:
  ret i64 0
}
";

fn calls_to<'context, 'module>(
    function: Value<'context, 'module, Function>,
    callee: Value<'context, 'module, Function>,
) -> usize {
    function
        .function_instructions()
        .filter_map(|instruction| instruction.as_kind::<CallBase>())
        .filter(|call| call.called_function() == Some(callee))
        .count()
}

fn function_named<'context, 'module>(
    module: &'module Module<'context>,
    name: &str,
) -> Value<'context, 'module, Function> {
    module.get_function(name).expect("the test's function")
}

fn run_recursive_inline(module: &mut Module<'_>, depth: u32, max_size: u32) -> u32 {
    let inlined = Cell::new(0);
    {
        let inlined = &inlined;
        module
            .run_passes_with("recursive-inline", None, &PassOptions::default(), move |builder| {
                builder.register_module_pass("recursive-inline", move || RecursiveInline {
                    depth,
                    max_size,
                    inlined: Some(inlined),
                });
            })
            .expect("the pipeline");
    }
    inlined.get()
}

#[test]
fn decision_1_recursive_inline_does_what_kayos_test_asks() {
    with_context(|context| {
        let mut module = context.parse_ir_text(RECURSIVE_INLINE_IR).expect("the test's IR");
        let inlined = run_recursive_inline(&mut module, 3, 1000);
        assert_eq!(module.verify(), Ok(()));
        let fib = function_named(&module, "fib");
        let countdown = function_named(&module, "countdown");
        let say = function_named(&module, "say");
        let kept = function_named(&module, "kept");
        let even = function_named(&module, "even");
        let odd = function_named(&module, "odd");
        // Three levels of two calls each: 2 + 4 + 8 inlined, 16 calls left.
        assert_eq!(calls_to(fib, fib), 16);
        assert_eq!(calls_to(countdown, countdown), 1);
        assert_eq!(calls_to(countdown, say), 8);
        assert_eq!(calls_to(kept, kept), 1);
        // Mutual recursion is joined: each partner inlined where the other calls it, so each calls itself, and is unfolded
        // three levels.
        assert_eq!(calls_to(even, odd), 0);
        assert_eq!(calls_to(odd, even), 0);
        assert_eq!(calls_to(even, even), 1);
        assert_eq!(calls_to(odd, odd), 1);
        assert_eq!(inlined, 2 + 4 + 8 + 3 + 2 + 3 + 3);
        assert_eq!(module.functions().count(), 6);
    });
}

#[test]
fn decision_1_recursive_inline_holds_the_size_limit() {
    with_context(|context| {
        let mut module = context.parse_ir_text(RECURSIVE_INLINE_IR).expect("the test's IR");
        let inlined = run_recursive_inline(&mut module, 3, 10);
        assert_eq!(inlined, 0);
        let fib = function_named(&module, "fib");
        assert_eq!(calls_to(fib, fib), 2);
    });
}

const HEAP_TO_STACK_IR: &str = r"
declare ptr @cyborg_alloc(i64)
declare void @cyborg_free(ptr)
declare void @cyborg_str_free(ptr, i64)
declare void @cyborg_out_i32(ptr)
declare void @escape(ptr)
declare void @llvm.memset.p0.i64(ptr, i8, i64, i1)

define i32 @stays_here() {
entry:
  %p = call ptr @cyborg_alloc(i64 16)
  %q = getelementptr i8, ptr %p, i64 4
  store i32 7, ptr %q
  %v = load i32, ptr %q
  call void @llvm.memset.p0.i64(ptr %p, i8 0, i64 4, i1 false)
  call void @cyborg_out_i32(ptr %p)
  call void @cyborg_free(ptr %p)
  ret i32 %v
}

define void @leaves() {
entry:
  %p = call ptr @cyborg_alloc(i64 16)
  call void @escape(ptr %p)
  call void @cyborg_free(ptr %p)
  ret void
}

define void @too_big() {
entry:
  %p = call ptr @cyborg_alloc(i64 8192)
  call void @cyborg_free(ptr %p)
  ret void
}

define void @zero_bytes() {
entry:
  %p = call ptr @cyborg_alloc(i64 0)
  call void @cyborg_free(ptr %p)
  ret void
}

define void @static_text(ptr %text) {
entry:
  call void @cyborg_str_free(ptr %text, i64 -9223372036854775803)
  ret void
}

define void @owned_text(ptr %text) {
entry:
  call void @cyborg_str_free(ptr %text, i64 5)
  ret void
}

define void @frame_budget() {
entry:
  %a = call ptr @cyborg_alloc(i64 4000)
  %b = call ptr @cyborg_alloc(i64 4000)
  call void @cyborg_free(ptr %a)
  call void @cyborg_free(ptr %b)
  ret void
}

define void @lend_keeps_nothing(ptr captures(none) %borrowed) {
  ret void
}

define void @lender() {
entry:
  %p = call ptr @cyborg_alloc(i64 8)
  call void @lend_keeps_nothing(ptr captures(none) %p)
  call void @cyborg_free(ptr %p)
  ret void
}
";

fn count_calls(function: Value<'_, '_, Function>, name: &str) -> usize {
    function
        .function_instructions()
        .filter_map(|instruction| instruction.as_kind::<CallBase>())
        .filter(|call| call.called_function().is_some_and(|callee| callee.name() == name))
        .count()
}

#[test]
fn decision_1_heap_to_stack_moves_what_stays_and_leaves_the_rest() {
    with_context(|context| {
        let mut module = context.parse_ir_text(HEAP_TO_STACK_IR).expect("the test's IR");
        module
            .run_passes_with("function(heap-to-stack)", None, &PassOptions::default(), |builder| {
                builder.register_function_pass("heap-to-stack", || HeapToStack);
            })
            .expect("the pipeline");
        assert_eq!(module.verify(), Ok(()));
        let stays_here = function_named(&module, "stays_here");
        assert_eq!(count_calls(stays_here, "cyborg_alloc"), 0);
        assert_eq!(count_calls(stays_here, "cyborg_free"), 0);
        assert!(stays_here.print_to_string().contains("%on_stack = alloca [16 x i8], align 16"));
        for kept in ["leaves", "too_big", "zero_bytes"] {
            let function = function_named(&module, kept);
            assert_eq!(count_calls(function, "cyborg_alloc"), 1, "{kept}");
            assert_eq!(count_calls(function, "cyborg_free"), 1, "{kept}");
        }
        assert_eq!(count_calls(function_named(&module, "static_text"), "cyborg_str_free"), 0);
        assert_eq!(count_calls(function_named(&module, "owned_text"), "cyborg_str_free"), 1);
        // One 4000-byte allocation fits the frame's 4096 bytes, and the second stays on the heap.
        let frame = function_named(&module, "frame_budget");
        assert_eq!(count_calls(frame, "cyborg_alloc"), 1);
        assert_eq!(count_calls(frame, "cyborg_free"), 1);
        assert_eq!(count_calls(function_named(&module, "lender"), "cyborg_alloc"), 0);
    });
}

fn plugin_path() -> std::path::PathBuf {
    // `cargo test --test kayo_passes` doesn't build examples, so this does, and it's quick if they are there already.
    static BUILT: std::sync::Once = std::sync::Once::new();
    BUILT.call_once(|| {
        let mut build = Command::new(env!("CARGO"));
        build.args(["build", "--example", "kayo_passes_plugin"]);
        if !cfg!(debug_assertions) {
            build.arg("--release");
        }
        let built = build.current_dir(env!("CARGO_MANIFEST_DIR")).output().expect("running cargo");
        assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    });
    // Tests run from target/<profile>/deps, and examples are built beside that.
    let mut path = std::env::current_exe().expect("this test's path");
    path.pop();
    path.pop();
    path.join(format!("examples/{}kayo_passes_plugin{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX))
}

fn opt_with_plugin(pipeline: &str, input: &str) -> String {
    let directory = std::env::temp_dir().join(format!("lyftar-plugin-{}-{pipeline}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a temporary directory");
    let input_path = directory.join("in.ll");
    std::fs::write(&input_path, input).expect("writing the IR");
    let opt = std::env::var_os("LYFTAR_LLVM_CONFIG").map_or_else(
        || std::path::PathBuf::from("/opt/homebrew/opt/llvm/bin/opt"),
        |config| std::path::Path::new(&config).with_file_name("opt"),
    );
    let output = Command::new(opt)
        .arg(format!("-load-pass-plugin={}", plugin_path().display()))
        .arg(format!("-passes={pipeline}"))
        .arg("-S")
        .arg(&input_path)
        .output()
        .expect("running opt");
    let _ = std::fs::remove_dir_all(directory);
    assert!(output.status.success(), "opt failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn decision_1_heap_to_stack_runs_through_opt_as_a_plugin() {
    let printed = opt_with_plugin("function(heap-to-stack)", HEAP_TO_STACK_IR);
    assert!(printed.contains("%on_stack = alloca [16 x i8], align 16"), "{printed}");
}

#[test]
fn decision_1_recursive_inline_runs_through_opt_as_a_plugin() {
    let printed = opt_with_plugin("recursive-inline", RECURSIVE_INLINE_IR);
    with_context(|context| {
        let module = context.parse_ir_text(&printed).expect("what opt printed");
        let fib = function_named(&module, "fib");
        assert_eq!(calls_to(fib, fib), 16);
    });
}
