# Lyftar

Lyftar builds LLVM IR from safe Rust and writes LLVM passes in it, for the latest stable LLVM. It does what inkwell and
llvm-plugin do, as one crate, and supports one LLVM at a time: LLVM 23 today.

The design is in [DECISIONS.md](DECISIONS.md), the rules for its code in [IMPLEMENTATION.md](IMPLEMENTATION.md), and what it
assumes about LLVM in [ASSUMPTIONS.md](ASSUMPTIONS.md).

## What it needs

LLVM 23 with its `llvm-config`, found on `PATH`, at Homebrew's `llvm`, or through `LYFTAR_LLVM_CONFIG`. `build.rs` stops with
a clear error on any other version. macOS uses the committed bindings. Linux and Windows generate theirs during the build and
need `libclang` for it. On Linux, a program using Lyftar also needs LLVM's library directory on its library path, for `libLTO`
and `libRemarks`, such as through `LD_LIBRARY_PATH`. Windows links LLVM statically, with the static C runtime: build with
`RUSTFLAGS="-C target-feature=+crt-static"`, use `clang-cl` as the C++ compiler, and have LLVM's `bin` on `PATH`, for `LTO.dll`
and `Remarks.dll`. The official download leaves out the static libraries it was built against, so put `zs.lib`,
`zstd_static.lib` and `xml2s.lib` in its `lib`, built with the static C runtime, such as vcpkg's `x64-windows-static`.

## Building a function

All work happens inside `with_context`, and nothing that belongs to the context can leave it. Plain data can.

You own an `OwnedModule`, and reach what's in it through the context: `context.module(&m)` to read and build, and
`context.exclusive_module(&mut m)` for what can delete things, such as running passes.

```rust
use lyftar::target::{CodeGenerationFileType, TargetMachine};
use lyftar::passes::PassOptions;
use lyftar::{Bits, Builder, Integer, with_context};

type I64 = Integer<Bits<64>>;

fn object_file() -> Vec<u8> {
    let machine = TargetMachine::for_host().expect("this machine has a target");
    with_context(|context| {
        let mut program = context.create_module("example");
        {
            let module = context.module(&program);
            module.set_target_machine(&machine);
            let i64_type = context.get_type::<I64>();
            let dynamic = i64_type.to_dynamic();
            let function = module.add_function("add_one", context.function_type(dynamic, &[dynamic], false));
            let builder = Builder::at_end_of(function.append_basic_block("entry"));
            let parameter = function.parameter(0).checked_cast::<I64>();
            let sum = builder.integer_add(parameter, i64_type.constant_integer(1, false), "sum");
            builder.return_value(sum);
            module.verify().expect("valid IR");
        }
        context
            .exclusive_module(&mut program)
            .run_passes("default<O2>", Some(&machine), &PassOptions::default())
            .expect("the pipeline");
        let buffer = machine
            .emit_to_memory_buffer(context.exclusive_module(&mut program), CodeGenerationFileType::Object)
            .expect("code generation");
        buffer.as_bytes().to_vec()
    })
}
```

A value says as much as the code knows. `Value<Integer<Bits<64>>>` is checked at `cargo build`, and a dynamic `Value` is
checked when the line runs. Types spell out what they are: `Pointer<AddressSpace<5>>`, `Array<Integer<Bits<32>>, Elements<4>>`,
`Struct<(Integer<Bits<32>>, Pointer)>`, and a function's signature, whose calls are checked at `cargo build`:

```rust
use lyftar::{Bits, Builder, Function, Integer, Value, Void, with_context};

type I32 = Integer<Bits<32>>;

with_context(|context| {
    let program = context.create_module("example");
    let module = context.module(&program);
    let add = module.add_specific_function::<Function<I32, (I32, I32)>>("add");
    let caller = module.add_specific_function::<Function<Void, ()>>("caller");
    let builder = Builder::at_end_of(caller.append_basic_block("entry"));
    let one = context.get_type::<I32>().constant_integer(1, false);
    let _sum: Value<'_, '_, I32> = builder.call(add, (one, one), "sum");
    builder.return_void();
    module.verify().expect("valid IR");
});
```

 A bad build call panics and names the problem, and what can really fail, such as reading a file,
returns an error.

Every value borrows the context, and an exclusive call needs the context to itself, so a value held across a delete or a pass
run on any module doesn't compile. That's because LLVM's passes can delete a constant, and LLVM keeps one copy of an identical
constant per context, so a pass on one module could otherwise delete what another module's code still holds. Types and plain data
(integers, floats, null, undef, poison, zero, data arrays and vectors) borrow nothing and survive. Aggregates and constant
expressions come from the context, as in `context.constant_struct(..)`, and can be shared across its modules. To keep a place in
a module across an exclusive call, use a tracked handle from `module.track(value)`.

## Writing a pass

A pass is a Rust type, with no C++ for its author. `examples/kayo/` has two real ones.

```rust
use lyftar::module::Module;
use lyftar::passes::{Analyses, ModulePass, PreservedAnalyses};

struct CountFunctions;

impl ModulePass for CountFunctions {
    fn run<'context>(&mut self, module: &mut Module<'context>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
        println!("{} functions", module.functions().count());
        PreservedAnalyses::All
    }
}
```

Linked into a compiler, it runs through `Module::run_passes_with`:

```rust
# use lyftar::module::Module;
# use lyftar::passes::{Analyses, ModulePass, PassOptions, PreservedAnalyses};
# struct CountFunctions;
# impl ModulePass for CountFunctions {
#     fn run<'context>(&mut self, _module: &mut Module<'context>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
#         PreservedAnalyses::All
#     }
# }
# fn run(module: &mut Module<'_>) -> Result<(), lyftar::error::Error> {
// `module` is what `context.exclusive_module(&mut program)` gives.
module.run_passes_with("count-functions", None, &PassOptions::default(), |builder| {
    builder.register_module_pass("count-functions", || CountFunctions);
})?;
# Ok(())
# }
```

As a plugin, a `cdylib` uses `lyftar::pass_plugin!` and loads with `opt -load-pass-plugin`. See
`examples/kayo_passes_plugin.rs`.

## Commands

Regenerate the committed macOS bindings when LLVM moves:

```bash
LIBCLANG_PATH=/opt/homebrew/opt/llvm/lib cargo run --manifest-path tools/bindings/Cargo.toml -- /opt/homebrew/opt/llvm/include src/raw/macos.rs
```

Update `coverage.json`, which lists every raw function and every inkwell API the owner's projects use:

```bash
cargo run --manifest-path tools/coverage/Cargo.toml
```

Run the AddressSanitizer and UBSan tests, which need nightly Rust:

```bash
LYFTAR_SANITIZE=1 RUSTFLAGS="-Zsanitizer=address" cargo +nightly test --target aarch64-apple-darwin --target-dir target/sanitize --test decision_2_end_to_end --test decision_3_panics --test decision_3_pass_aborts --test decision_4_build_calls --test decision_10_jit --test decision_11_deletion --test decision_1_passes --test decision_16_modules --test decision_16_threads --test decision_30_bytes --test decision_31_pointers --test decision_32_aggregates --test decision_33_functions --test decision_43_modules --test kayo_passes --no-fail-fast -- --skip plugin
```

Measure what Lyftar adds over raw LLVM calls, beside the saved table, and save a new one with `-- --save`:

```bash
cargo bench --bench overhead
```

`cargo test` also runs Verus on the proven checks in `src/checks.rs`, so working on Lyftar on a Mac needs
[Verus](https://github.com/verus-lang/verus) at `~/.local/verus/verus-arm64-macos/verus` or named by `LYFTAR_VERUS`. Using
Lyftar as a dependency needs none of it.

## Licenses

Code is under `Apache-2.0 WITH LLVM-exception` ([LICENSE](LICENSE)), and written docs are under CC BY 4.0
([LICENSE-CC-BY-4.0](LICENSE-CC-BY-4.0)).
