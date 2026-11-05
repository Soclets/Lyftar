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

## Waiting on design claude

Nothing. D30 to D35 are built. Told design claude what they leave to the code: 12 as the fixed maximum for specific calls and
structs, `add_specific_function`, `builder.call` taking a tuple or a slice, `BasicBlockAsValue` for the marker, and constants of
specific aggregates going through `checked_cast`.

## Untested

- Linux and Windows, and `.github/workflows/ci.yml`, which has not run yet.

## kayo's passes

`examples/kayo/` in Rust, compared with kayo's C++ passes on the same IR through `opt`: the printed output was byte-identical
for both passes (a one-off in scratch, since the other repository is a read-only reference).
