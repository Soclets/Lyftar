# Lyftar

Lyftar builds LLVM IR from safe Rust and writes LLVM passes in it, for the latest stable LLVM. It does what inkwell and
llvm-plugin do, as one crate, and supports one LLVM at a time: LLVM 23 today.

The design is in [DECISIONS.md](DECISIONS.md), the rules for its code in [IMPLEMENTATION.md](IMPLEMENTATION.md), and what it
assumes about LLVM in [ASSUMPTIONS.md](ASSUMPTIONS.md).

## What it needs

LLVM 23. By default `build.rs` downloads Lyftar's own static LLVM and lld for the host from this repository's releases, once, into
the user's cache directory, and checks it against the hash it carries (D50). These exist for macOS on Apple Silicon and Linux on ARM
and x86-64, and need nothing else installed. To use an LLVM of your own, such as Homebrew's, set `LYFTAR_LLVM_CONFIG` to its
`llvm-config`. `build.rs` stops with a clear error on any other version. macOS uses the committed bindings. Linux and Windows generate theirs during the build and
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
`examples/kayo_passes_plugin.rs`. On Windows that's a compile error, because the official LLVM there is built with plugins off;
passes linked into a compiler work there.

## Garbage collection

A language on Lyftar can have a precise, moving collector. `function.set_garbage_collector` names a strategy, and a strategy built on
statepoints, such as `statepoint-example`, wants its pointers in address space 1. Run `rewrite-statepoints-for-gc` after anything
that inlines, which `statepoint_pipeline` writes for you:

```rust
use lyftar::garbage_collection::{SafepointPlacement, statepoint_pipeline};
use lyftar::passes::PassOptions;
use lyftar::{AddressSpace, Builder, Function, Pointer, Void, with_context};

type Managed = Pointer<AddressSpace<1>>;

with_context(|context| {
    let mut program = context.create_module("example");
    let module = context.module(&program);
    let allocate = module.add_specific_function::<Function<Managed, ()>>("allocate");
    let consume = module.add_specific_function::<Function<Void, (Managed,)>>("consume");
    let run = module.add_specific_function::<Function<Void, ()>>("run");
    run.set_garbage_collector("statepoint-example");
    let builder = Builder::at_end_of(run.append_basic_block("entry"));
    let first = builder.call(allocate, (), "first");
    let second = builder.call(allocate, (), "second");
    builder.call(consume, (first,), "");
    builder.call(consume, (second,), "");
    builder.return_void();
    drop(builder);
    let pipeline = statepoint_pipeline("default<O2>", SafepointPlacement::CallsOnly);
    context
        .exclusive_module(&mut program)
        .run_passes(&pipeline, None, &PassOptions::default())
        .expect("the pipeline");
    assert!(context.module(&program).print_to_string().contains("gc.statepoint"));
});
```

The code generator then writes a stack map saying where each pointer is at each call. A compiled program reads it with the
`lyftar-stack-maps` crate, which has no LLVM, so the program needn't link it. Looking a return address up allocates nothing, so it can
run during a collection. `Jit::stack_map_sections` and `ExecutionEngine::stack_map_sections` give the same bytes for code made in
memory. Keep the section when linking: no `-dead_strip` or `--gc-sections`, and with rustc, `-C link-dead-code`. On Linux the section is
read-only and holds addresses, so a position-independent program also needs `-Wl,-z,notext`, or to be linked with `-no-pie`.

Rewrite each root at a call in three steps, since one slot can be in several pairs: turn each derived pointer into its offset while
every base is still old, move each base, then add the offset back. A stack map only covers the compiled program's stack, so a
runtime's own temporaries need roots of their own, and an integer never reaches a stack map, so nothing rewrites one.

`Builder::garbage_collection_statepoint` writes statepoints by hand, with `garbage_collection_relocate` and
`garbage_collection_result`. `shadow-stack`, `ocaml` and `erlang` work from `garbage_collection_root` instead, and `erlang` only for
ELF. A strategy of your own is a `GarbageCollectionStrategy` in Rust, given to `register_strategy`, linked in or in a plugin.
`tests/collector/` is a small moving collector that runs against all of these.

`ocaml`'s frame table can only be read on Darwin on ARM and on x86-64. LLVM gives each root's offset from the frame pointer or the
stack pointer without saying which. On Linux on ARM a root can sit above the frame pointer, where the two look the same. Statepoints
and `shadow-stack` name their registers, so they work there (D54).

## A self-contained compiler

A compiler built on Lyftar can build and link for macOS on Apple Silicon, Linux on ARM and x86-64, and a Cortex-M4 with nothing
else installed: no LLVM, no linker and no SDK (D49). With the default, downloaded LLVM, lld and the system files each target needs
are built into Lyftar, so the compiler is one file that needs only the system's own libraries, of about 75 MB, or 115 MB if it
compiles C (see below). Link with `lyftar::link`:

```rust,no_run
use std::path::Path;
use lyftar::link;
use lyftar::linking::SystemTarget;

# fn run() -> Result<(), lyftar::error::Error> {
link(SystemTarget::LinuxAarch64, &[Path::new("program.o")], Path::new("program"), None)?;
link(SystemTarget::CortexM4, &[Path::new("firmware.o")], Path::new("firmware.elf"), Some(Path::new("memory.ld")))?;
# Ok(())
# }
```

- macOS programs link against `libSystem`, through a stub Lyftar writes from the symbols the objects need, so no file from Apple is
  involved. They run on macOS 11 and later.
- Linux programs are linked statically with musl and compiler-rt's builtins into one file that runs on any Linux of that processor.
  Programs that need glibc aren't supported.
- Bare metal gets compiler-rt's builtins. Its startup code and linker script are the front end's own.
- `lyftar::lld` runs lld with its own arguments, for anything else. Links in one process run one at a time, since two at once crash
  lld.
- Licences: a compiler with LLVM inside ships LLVM's licence text, and a Linux program carries musl's MIT notice. Both are in the
  downloaded files, as `LICENSE-LLVM.txt`, `LICENSE-lld.txt`, `LICENSE-musl.txt` and `LICENSE-compiler-rt.txt`.
- Plugins need a shared LLVM, so `pass_plugin!` is a compile error with the downloaded one (D53).
- With an LLVM of your own through `LYFTAR_LLVM_CONFIG`, `link` and `lld` work only if its library folder has lld's libraries.
  Homebrew's LLVM doesn't.

### Compiling C

The same compiler can compile C for those four targets, with clang inside Lyftar (D55), such as a maths library like CORE-MATH.
`lyftar::compile_c` compiles one C file to an object, which `link` then links:

```rust,no_run
use std::path::Path;
use lyftar::compile_c;
use lyftar::compiling::CCompileOptions;
use lyftar::linking::SystemTarget;

# fn run() -> Result<(), lyftar::error::Error> {
let headers = Path::new("my-libc/include");
let options = CCompileOptions { include_folders: &[headers], defines: &[("NDEBUG", None)], ..CCompileOptions::default() };
let warnings = compile_c(SystemTarget::CortexM4, Path::new("sin.c"), Path::new("sin.o"), &options)?;
print!("{warnings}");
# Ok(())
# }
```

- Only C, and only to an object. There's no C++ and no linking: `link` does that.
- `-O2` unless the options say otherwise (D57), and macros as `-D` gives them (D58).
- clang's own headers, the ones C on ARM and x86 can reach (`stdint.h`, `stddef.h`, `arm_neon.h`, `immintrin.h` and the rest), are
  built in, as they are, about 11 MB (D60). No system folder is searched, so libc's headers (`math.h`, `fenv.h`, `errno.h`) are
  yours, through `include_folders`, which are searched first.
- The Cortex-M4 is compiled for with its hard-float unit, as `link` links it. clang 23 has C23's `_BitInt(128)`, which works on its
  32-bit processor where `__int128` doesn't.
- What clang printed comes back: its warnings when the object was written, and its errors as the error when it wasn't. Compiles
  can run at once on several threads.
- clang's licence is in the downloaded files as `LICENSE-clang.txt`. With an LLVM of your own, `compile_c` works only if it is static
  and its library folder has clang's libraries.

What clang costs, measured on macOS on Apple Silicon with release builds, stripped (D61):

| | Size |
|---|---|
| A compiler that links, built on a Lyftar without clang | 66.0 MB |
| The same compiler on this Lyftar, never calling `compile_c` | 74.6 MB |
| A compiler that also calls `compile_c` (`examples/self_contained.rs`) | 115.0 MB |
| The downloaded LLVM, per host, with clang | 102 to 150 MB, 17 to 22 MB more than without |

A compiler that never compiles C still carries 8.6 MB of clang: Rust puts `compile_c`'s code in the same object file as code every
compiler uses, and once that file is linked, clang's start-up code keeps part of clang. Calling `compile_c` adds the rest, about
34 MB of clang and 11 MB of its headers.

`tests/decision_49_self_contained.rs` builds `examples/self_contained.rs`, which compiles C and links, and runs it where it can't
read any LLVM, lld, clang, SDK or developer tools.

## Commands

Regenerate the committed macOS bindings when LLVM moves:

```bash
LIBCLANG_PATH=/opt/homebrew/opt/llvm/lib cargo run --manifest-path tools/bindings/Cargo.toml -- /opt/homebrew/opt/llvm/include src/raw/macos.rs
```

Update `coverage.json`, which lists every raw function and every inkwell API the owner's projects use:

```bash
cargo run --manifest-path tools/coverage/Cargo.toml
```

Run the AddressSanitizer and UBSan tests, which need nightly Rust. The shim is built with Homebrew's Clang, whose AddressSanitizer is
the version Rust's is. LLVM's own libraries aren't instrumented, and its allocators mark memory with AddressSanitizer from the
instrumented shim's copies of their code, which clang's code then uses through its own copies, so those marks are off:

```bash
CXX=/opt/homebrew/opt/llvm/bin/clang++ ASAN_OPTIONS=allow_user_poisoning=0 LYFTAR_SANITIZE=1 RUSTFLAGS="-Zsanitizer=address" cargo +nightly test --target aarch64-apple-darwin --target-dir target/sanitize --test decision_2_end_to_end --test decision_3_panics --test decision_3_pass_aborts --test decision_4_build_calls --test decision_10_jit --test decision_11_deletion --test decision_1_passes --test decision_16_modules --test decision_16_threads --test decision_30_bytes --test decision_31_pointers --test decision_32_aggregates --test decision_33_functions --test decision_43_modules --test decision_45_moving_collection --test decision_45_statepoints --test decision_45_strategies --test decision_45_stack_maps --test decision_45_custom_strategies --test decision_55_compile_c --test kayo_passes --no-fail-fast -- --skip plugin
```

Measure what Lyftar adds over raw LLVM calls, beside the saved table, and save a new one with `-- --save`:

```bash
cargo bench --bench overhead
```

`cargo test` also runs Verus on the proven checks in `src/checks.rs` and `stack-maps/src/checks.rs`, so working on Lyftar on a Mac needs
[Verus](https://github.com/verus-lang/verus) at `~/.local/verus/verus-arm64-macos/verus` or named by `LYFTAR_VERUS`. Using
Lyftar as a dependency needs none of it.

## Licenses

Code is under `Apache-2.0 WITH LLVM-exception` ([LICENSE](LICENSE)), and written docs are under CC BY 4.0
([LICENSE-CC-BY-4.0](LICENSE-CC-BY-4.0)).
