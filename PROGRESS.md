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

## Known hole: context constants deleted by passes

Reproduced with `#![forbid(unsafe_code)]`, under AddressSanitizer: heap use-after-free, allocated by `constant_struct` and freed by
`EliminateAvailableExternallyPass`. Make `held = context.constant_struct(..)`. Add a global with `held` as its initializer and
`Linkage::AvailableExternally`, and a function that loads it. Run `default<O2>`, then read `held`, say with `module.track(held)`.
`Linkage::Internal` with no use and `globaldce` does the same.

Also found while probing, and fixed under D38: `default<Os>`, `<Oz>`, `function-import`, `free-machine-function` and text with
debug information that fails the verifier now give errors. Not fixed: bitcode with debug information that fails the verifier
still ends the process, in a program with no `unsafe`. LLVM's reader has no switch, and verifying first would refuse valid old
bitcode whose intrinsics aren't upgraded yet.

Probing all of LLVM's named passes with a `WeakVH` on each constant found these deleters and no others. A hidden module holding
a real use of the constant stopped the deletion in the two cases tried, with the same printed IR. The fix is waiting on the owner.

## Waiting on design claude

D39 is not built. The token is tried in scratch and does what it says for constants made through it, but a constant read back out
of a module borrows only that module, and ASan shows it dangling after a pass on another module: `global.initializer()` then
`remove_initializer()`, or `node_operands()` from metadata, with no removal at all. Design claude has the options, the spelling
and the proposal of a separate `Constants` type. Probes found no deletion from dropping, cloning, linking, stripping debug
information, erasing or a bitcode round trip, and none from emission on 17 targets or MCJIT, though those run passes that can.
Where `ConstantPtrAuth` and the unlisted plain-data kinds go is also open.

## Untested

- Linux and Windows, and `.github/workflows/ci.yml`, which has not run yet.

## kayo's passes

`examples/kayo/` in Rust, compared with kayo's C++ passes on the same IR through `opt`: the printed output was byte-identical
for both passes (a one-off in scratch, since the other repository is a read-only reference).
