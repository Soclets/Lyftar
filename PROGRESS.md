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

## Waiting on design claude

- Spelling of: `Byte<Bits<N>>`, pointer address spaces, specific arrays/vectors/structs, function signatures. All of these
  work through dynamic `Type`/`Value` until answered.
- `replace_all_uses_with` takes a shared borrow, and LLVM would delete constants that use the value, or metadata around it, in
  some cases. It panics there. Whether there should be an exclusive form for those cases is the owner's call; it is unbuilt.

## Untested

- Linux and Windows, and `.github/workflows/ci.yml`, which has not run yet.

## kayo's passes

`examples/kayo/` in Rust, compared with kayo's C++ passes on the same IR through `opt`: the printed output was byte-identical
for both passes (a one-off in scratch, since the other repository is a read-only reference).
