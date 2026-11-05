# Progress

Freeform notes for whoever picks this up after a compaction. `coverage.json` holds coverage and git holds history.

## Findings

- Verus: `#[cfg_attr(verus_keep_ghost, verus_spec(...))]` with specs in a `#[cfg(verus_keep_ghost)] verus! { }` block needs
  nothing from a normal build, so users of Lyftar pull in no Verus crate. Verus is at `~/.local/verus` with toolchain 1.99.1.
- `getTerminator()` asserts on an unterminated block in LLVM 23: use `hasTerminator()` or `getTerminatorOrNull()`.

## Waiting on design claude

Spelling of: `Byte<Bits<N>>`, pointer address spaces, specific arrays/vectors/structs, function signatures. All of these
work through dynamic `Type`/`Value` until answered.
