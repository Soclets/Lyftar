# Lyftar — design decisions

The owner decides; design claude proposes and records. Each entry says what was chosen and whose idea it was.
Numbers are permanent, so a decision is never renumbered. A changed decision gets a new entry that says which one it
replaces.

## Decided

**D1. What Lyftar is.** Lyftar is what inkwell and llvm-plugin are, for the latest stable LLVM: building LLVM IR from
safe Rust, and writing LLVM passes in it. (Owner, 2026-11-04)

**D2. One LLVM at a time.** Lyftar supports one LLVM version, the latest stable, with no per-version feature flags.
When a new major version ships, `main` moves to it and drops the old one; anyone who needs the old LLVM stays on an
older Lyftar. Claude proposed it; the owner chose it: "yes, because that's what my langs want". (Owner, 2026-11-04)
