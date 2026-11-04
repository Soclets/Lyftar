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

**D4. One generic `Value<T>`.** A value is a `Value<T>`, as specific as the code knows: a type written into the Rust
code is checked at `cargo build`, and a type that comes from the program being compiled is a dynamic `Value`, checked
when the line runs and panicking as in D3. A checked cast moves a value from dynamic to specific, and a pass can ask
about a type and get an `Option`. The owner asked for the best design with the writing pain ignored; Claude proposed
this; the owner chose it: "yes". The spelling of the type names is not settled here. (Owner, 2026-11-04)

**D5. Full words, no exceptions.** Names in Lyftar's API are full words: `Integer`, `Pointer`, `Dynamic`,
`integer_add`, `zero_extend`, and `get_element_pointer` rather than `gep`. Claude proposed the rule and asked whether
`gep` should be an exception; the owner chose: "yes, and no". (Owner, 2026-11-04)

**D6. An integer's width says its unit.** The unit is bits, and the name shows it: "Where's the unit? Bits". The
spelling is not settled here. (Owner, 2026-11-04)

**D7. `Integer<Bits<64>>`.** A specific integer is spelled `Integer<Bits<64>>`, so its value is
`Value<Integer<Bits<64>>>`. This settles the spelling D6 left open. Claude proposed it; the owner chose it: "yes".
(Owner, 2026-11-04)

**D8. Every number says its unit.** Every number in Lyftar's API says its unit, such as `size_in_bits`,
`store_size_in_bytes` and `alignment_in_bytes`. Claude proposed it; the owner chose it: "yes to both". (Owner,
2026-11-04)

**D9. Floats say their format.** Every floating-point type is a `Float<F>`, where `F` says the format. IEEE 754's
binary formats say the standard, the base and the width: `half` is `Float<Ieee754Binary<Bits<16>>>`, `float` is
`Float<Ieee754Binary<Bits<32>>>`, `double` is `Float<Ieee754Binary<Bits<64>>>`, and `fp128` is
`Float<Ieee754Binary<Bits<128>>>`. The others are `Float<BrainFloat>` for `bfloat`, `Float<X87Extended>` for `x86_fp80`
and `Float<PowerPcDoubleDouble>` for `ppc_fp128`. `Ieee754` is the standard's name and counts as a word under D5. The
owner asked for the standard and then the base: "Shouldn't it say the standard too? IEEE 754, right?" and "Bro, where's
binary?" Claude proposed the rest. (Owner, 2026-11-04)

**D10. A context lives in a closure.** LLVM work happens inside `with_context(|context| ...)`, which gives each context
its own lifetime. Using a value with another context, or taking one out of the closure, does not compile; plain data
such as object file bytes can be returned. A long-lived JIT runs inside the closure or takes a fresh context per
compile. Claude proposed it; the owner chose it: "opt 2". (Owner, 2026-11-04)

**D11. The compiler enforces deletion.** A value that belongs to a module borrows it, and deleting from the module or
running passes on it needs exclusive access, so a value held across either does not compile. Types and constants belong
to the context and survive. Two pieces cover passes that keep lists: erasing several values at once, and a tracked
handle that survives changes and says whether its value is still there, built on LLVM's `WeakVH`. Claude proposed it;
the owner chose it: "enforce". (Owner, 2026-11-04)
