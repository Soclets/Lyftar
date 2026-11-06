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

## Waiting on design claude

Nothing. D30 to D43 are built.

## CI

`.github/workflows/ci.yml` runs on every push. Read it with `gh run list`: don't call Linux or Windows untested without looking.
Every run up to 6db7931 failed. Now:

- Linux and the macOS check pass on GitHub. The Verus proofs and the shim's `clang-format` and `clang-tidy` checks run on the Mac only.
- Windows builds and passes, with `pass_plugin!` a compile error and the two plugin tests off there (D44). `decision_2_end_to_end`,
  `decision_3_pass_aborts` and the program-linking test in `decision_33_functions` are `cfg(unix)`, so they don't run there either.

## kayo's passes

`examples/kayo/` in Rust, compared with kayo's C++ passes on the same IR through `opt`: the printed output was byte-identical
for both passes (a one-off in scratch, since the other repository is a read-only reference).
