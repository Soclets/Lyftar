# Progress

Freeform notes for whoever picks this up after a compaction. `coverage.json` holds coverage and git holds history.

## Findings

- Verus: `#[cfg_attr(verus_keep_ghost, verus_spec(...))]` with specs in a `#[cfg(verus_keep_ghost)] verus! { }` block needs
  nothing from a normal build, so users of Lyftar pull in no Verus crate. Verus is at `~/.local/verus` with toolchain 1.99.1.
- `getTerminator()` asserts on an unterminated block in LLVM 23: use `hasTerminator()` or `getTerminatorOrNull()`.

## Soundness notes

Things found by reading LLVM against the API, each with a guard: `LLVMGetRelocationTypeName` leaves off the NUL, the C API's
casts are unchecked in a release build, a `uses()` on a context constant reaches other modules, LLVM can delete an unresolved
metadata node when an operand changes, `DbgRecord::print` crashes on a record in a block with no terminator, and
`AtomicOrdering` and friends fall into `llvm_unreachable` on a value they do not know.

Found by the positive tests: erase called a phi entry from a block in its own list a blocker; the alias analysis text dangled,
since LLVM keeps the pointer; `LTOModule` keeps a reference to its bytes, and modules from another `libLTO` context could be
added to a generator. `ASSUMPTIONS.md` holds what these rest on.

Found when a run hung: LLVM's target registry and `libLTO` take no lock, so first uses on several threads could tie the list of
targets into a loop. A `libLTO` generator in the global context also leaves a dangling handler there. `set_up_targets`, the
lock in `lto.rs`, and generators that own their context are what came of it, and `tests/decision_16_threads.rs` reproduces the
hang without them.

## How the constant hole was closed

LLVM's passes can delete aggregates and constant expressions that belong only to the context, which was a use-after-free in safe
code through plain `default<O2>`. D37's borrow of one module and D39's token each left a way through, shown under AddressSanitizer:
a second module sharing the constant, and a constant read back from a module. D42 and D43 closed it, and each of those programs is
now a compile-fail test with a twin.

Also found while probing, and fixed under D38: `default<Os>`, `<Oz>`, `function-import`, `free-machine-function` and text with
debug information that fails the verifier now give errors. Not fixed: bitcode with debug information that fails the verifier
still ends the process, in a program with no `unsafe`. LLVM's reader has no switch, and verifying first would refuse valid old
bitcode whose intrinsics aren't upgraded yet.

Probing all of LLVM's named passes with a `WeakVH` on each constant found these deleters and no others. A hidden module holding
a real use of the constant stopped the deletion in the two cases tried, with the same printed IR. The fix is waiting on the owner.

## Garbage collection (D45, D46)

Built: all five strategies, statepoints by hand and by pass, `place-safepoints`, `lyftar-stack-maps` (`stack-maps/`), stack maps from
both JITs, and strategies in Rust. `tests/decision_45_moving_collection.rs` runs a moving collector (`tests/collector/`) over
`statepoint-example`, `coreclr`, `shadow-stack`, `ocaml` and a Rust strategy, linked and through both JITs, optimized and not.
`erlang` only emits: its table goes in `.note.gc`, which isn't loaded, and its 4-byte addresses don't suit a position-independent
program, so a runtime would have to read the file. The test doesn't run it.

Found by real runs, each now an error or a panic rather than the process ending: a strategy name nothing registered, `gc.relocate`
in a function with no collector, the transition flag anywhere but x86 (`Cannot select`), `erlang` on Mach-O (memory corruption, seen
as an abort about one run in five), and `place-safepoints` with no poll. A static LLVM drops the `ocaml` and `erlang` table printers
without a call into each, which Windows' CI found. `remarks::version` had the same shape of problem earlier.

Found by the collector itself, since it keeps poisoned spaces rather than freeing them: `rewrite-statepoints-for-gc` adds nothing to a
statepoint it finds, so hand-written ones mustn't be inlined into a function it rewrites, and the optimizer drops a stack allocation in a
`gc-live` list, so those go after optimizing. Both are in `StatepointCall`'s docs. Linux's linker needs `-z notext` for a position-
independent program, since ELF's stack map section is read-only and holds addresses. Through `Jit`, `ocaml` needs an object file and
RuntimeDyld, and `shadow-stack` needs the program to define `llvm_gc_root_chain` when there are several translation units.

## Self-contained, as Zig is (D49): what the prototypes found

Prototypes are in `/Users/ts/llvm-self-contained` (outside the repository, since the scratchpad gets cleared): LLVM 23.1.2's source,
a static build of it with lld in `install/`, musl and compiler-rt builtins, and `prototype/` with the programs below.

- A source build with zlib, zstd, libxml2 and z3 off took about 10 minutes on 2 cores here, and needs only `-lm`. Homebrew can't
  give this: its static LLVM wants a shared `libz3` with no static copy, and its lld is shared only, at 23.1.0.
- A Rust program with that LLVM and lld inside is 70 MB, 58 MB stripped, and links only `libSystem` and `libc++`. It linked a macOS
  program, a Linux ARM `printf` program and a Cortex-M4 image in one process, inside `sandbox-exec` denying Homebrew, the build,
  Xcode and the SDK (`prototype/deny.sb`). The macOS program ran there; the Linux one ran in Alpine and busybox under colima.
- macOS needs no Apple file: a `libSystem.tbd` of 12 lines, naming the install name and the symbols called, is enough.
- Linux on ARM: musl 1.2.5 static (2.3 MB `libc.a`) and compiler-rt builtins (364 KB), which `printf` needs for 128-bit floats.
  Cortex-M4: builtins only (160 KB).
- lld runs again in one process, even after a failed link (`canRunAgain` stayed true). Two links at once crashed it (SIGTRAP).
- libLTO and libRemarks only come shared, so a self-contained build has to compile them as static libraries itself.
- The proposal went to design claude on 2026-11-10, and Tankun decided D50 to D53.

Built: `llvm.yml` publishes static LLVM and lld 23.1.2 for three hosts and the system files (release `llvm-23.1.2`); `build.rs`
downloads and checks them by default; `lyftar::lld` and `lyftar::link`; `pass_plugin!` refused with the static LLVM (D53).
`tests/decision_49_self_contained.rs` runs `examples/self_contained.rs` in `sandbox-exec`, denying the LLVM it was built with,
Homebrew, the download cache, Xcode and the command line tools: it links all four targets, twice for the Cortex-M4, the macOS program
runs, and the Linux one runs in Docker when colima is up (`colima start --arch aarch64`). Local runs use the static build in
`/Users/ts/llvm-self-contained/distribution`, the same recipe as the release, through `LYFTAR_LLVM_CONFIG`.

CI's first runs with the downloaded LLVM found three things, fixed in 0d24036: Windows hung untarring the system files (now only
fetched on D50's hosts); RuntimeDyld on x86-64 couldn't find `DW.ref.__gcc_personality_v0` with a position-independent machine (it
keeps LLJIT's own there); and the test collector read `ocaml` roots from the wrong place on Linux on AArch64, whose frame record sits
below the callee-saved registers (it now finds each caller's stack by the frame's size). Linux on ARM then showed that an `ocaml`
frame table can't say which register an offset is from there; Tankun chose to read it on Darwin on ARM and x86-64 only (D54).
Then RuntimeDyld couldn't find `DW.ref.__gcc_personality_v0` on Linux on ARM: Lyftar's RuntimeDyld layer now claims the weak
symbols code generation adds (3c0e2c7). CI passes on all six jobs at 3c0e2c7, and the self-contained test runs and passes on
GitHub's macOS, Linux on ARM and Linux on x86-64. D49 is done.

## Compiling C with clang inside (D55): what the prototype found

A scratch program linked clang's libraries statically with the same static LLVM as D50 (no RTTI; AArch64, ARM and X86), built in
`/Users/ts/llvm-self-contained/build-distribution` with `clang` added to the projects (about 5 minutes on 3 cores). It turns driver
arguments into a compile job with clang's `createInvocation`, which always adds `-fsyntax-only`, so `-c` and `-o` are taken out and
the output file is set on the job, then runs `EmitObjAction` in-process.

- CORE-MATH at 018fa97: `sin`, `cos`, `exp`, `log`, `pow`, `atan2` and `hypot` compile for the Cortex-M4 (`thumbv7em-none-eabihf`,
  `cortex-m4`, hard float, `fpv4-sp-d16`), macOS on ARM, and Linux on ARM and x86-64 (musl), at -O2. The disassembly is identical to
  Homebrew's `clang` 23.1.2 given the same arguments. The Cortex-M4 `sin` needs only compiler-rt's `__aeabi_d*` and `fma` and `round`.
- With `-nostdlibinc`, the job searches clang's own headers and the caller's `-I` folders, nothing else, on every target. Libc
  headers are the caller's: the test used small stubs of `fenv.h`, `errno.h`, `math.h`, `inttypes.h`, `stdlib.h` and `stdio.h`.
- Sixteen compiles on sixteen threads in one process, each with its own `CompilerInstance`, gave the same bytes as one at a time.
  Unlike lld, it needs no lock. (That run briefly used more than rule 11's 3 cores.)
- Size: clang adds 34 MB to a stripped compiler (`clangSema` 9.1, `clangAST` 7.4, `clangCodeGen` 5.1, `clangBasic` 3.8 MB). Its
  21 libraries and C++ headers add 16 MB to each host's `.tar.xz`. clang's resource headers are 16 MB; the ones C on ARM and x86 can
  reach are 200 files, 11 MB, or 0.5 MB compressed.

## Waiting on design claude

Nothing. D30 to D46 are built.

## CI

`.github/workflows/ci.yml` runs on every push. Read it with `gh run list`: don't call Linux or Windows untested without looking.
All three jobs pass at 58ffba3.

- The Verus proofs, the shim's `clang-format` and `clang-tidy` checks and the sanitizer run are on the Mac only.
- Linux runs everything else, including the moving collector linked and through both JITs, and the transition flag, which only x86
  can make code for.
- Windows builds and passes, with `pass_plugin!` a compile error and the two plugin tests off there (D44). These are `cfg(unix)`, so
  they don't run there: `decision_2_end_to_end`, `decision_3_pass_aborts`, the program-linking test in `decision_33_functions`, and
  `decision_45_moving_collection` and `decision_45_stack_maps`. `lyftar-stack-maps` has no `loaded_sections` for Windows (D44's scope).

## kayo's passes

`examples/kayo/` in Rust, compared with kayo's C++ passes on the same IR through `opt`: the printed output was byte-identical
for both passes (a one-off in scratch, since the other repository is a read-only reference).
