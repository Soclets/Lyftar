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

**D3. A bad build call panics.** Build calls return their value, not a `Result`. A builder only exists placed in a
block, so an unset position cannot happen. A request LLVM would turn into invalid IR, such as adding values of two
widths, panics at the call and names the problem; there is no checked variant. Inside a Rust pass the panic aborts the
compiler, since the IR may be half rewritten. This covers bugs in the compiler only: what can really fail at run time,
such as reading a bitcode file, still returns an error. Claude proposed it; the owner chose it: "Tbh, do we really want
a bug to keep running?" (Owner, 2026-11-04)
