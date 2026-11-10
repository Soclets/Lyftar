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

**D12. Our own bindings.** Lyftar generates its own raw bindings to LLVM's C API with `bindgen`, rather than depending
on `llvm-sys`, so a new LLVM never waits on another crate's release. Lyftar owns its linking and build setup. Claude
suggested `llvm-sys`; the owner chose otherwise: "our own". (Owner, 2026-11-04)

**D13. The bindings are committed.** The generated bindings live in the repository and a command regenerates them when
LLVM moves, so users need neither `libclang` nor `bindgen`, and each upgrade shows as a diff of the C API. If one file
cannot fit every platform, each platform gets its own, with CI checking they are current. `build.rs` checks that the
LLVM it finds is the supported version and stops with a clear error if not. Claude proposed it; the owner chose it:
"opt 1". (Owner, 2026-11-04)

**D14. Mac commits, the others generate.** This narrows D13. macOS uses the committed bindings, so it needs neither
`libclang` nor `bindgen`. Every other platform generates its bindings during the build, and needs `libclang` there.
Claude suggested macOS and Linux with committed bindings and leaving Windows for later; the owner chose: "Mac, and the
others do opt 2". (Owner, 2026-11-04)

**D15. Raw access in `unsafe`.** Each Lyftar type has `unsafe fn as_raw()` and `unsafe fn from_raw()`, and the raw
bindings are public as `lyftar::raw`, with no feature flag in front of them. Raw code can break what D11 guarantees,
and the `unsafe` marks where. Claude proposed it; the owner chose it: "opt 1". (Owner, 2026-11-04)

**D16. One crate.** Lyftar is one crate, `lyftar`, and the C++ shim always compiles. Its modules follow how LLVM divides
its C API, such as `lyftar::debug_info`, `lyftar::passes` and `lyftar::jit`. The owner asked what very specific crates
would make; Claude mapped them and suggested one crate; the owner chose it: "How about one?" (Owner, 2026-11-04)

**D17. Licenses as in Radium.** Code is under `Apache-2.0 WITH LLVM-exception`, the same terms as LLVM's headers, and
written docs are under CC BY 4.0. Claude proposed it, following Radium's D65; the owner chose it: "same". (Owner,
2026-11-04)

**D18. Versions are the publish date and time.** A version is `date.time.0`, such as `20261104.1430.0` for 14:30 UTC on
4 November 2026, with no leading zero on the time, as Cargo requires. The number does not say which LLVM; `build.rs`
checks that (D13). Cargo treats each new date as breaking, so a dependency is edited by hand for each day's release, and
two releases on one day count as compatible. Claude suggested putting `2300` first to keep automatic updates; the owner
chose otherwise: "The one that can break too. I mean, it's for me anyways". Reading this as the date-first scheme is
Claude's. (Owner, 2026-11-04)

**D19. Normal versions.** This replaces D18. Versions are ordinary ones, starting at `0.1.0`, and moving to a new LLVM
counts as a breaking change. The number does not say which LLVM; `build.rs` checks that (D13). After date-based versions
in both orders ran into Cargo's rules, the owner chose: "Fuck it, let's just do normal". Reading "normal" as `0.1.0`
rather than the date in y/m/d order is Claude's. (Owner, 2026-11-05)

**D20. A hook prints the rules.** The implementation rules live in one short file, `IMPLEMENTATION.md`, and a
`SessionStart` hook prints it into every session in this repository at startup, resume, clear and after compaction, the
implementation session included. The hook is copied from Cyborg's, which took the idea from TAKL. The rules themselves
are not settled here. Claude reported the idea from Cyborg's design session; the owner chose it: "Add the hook for this
session, and evnetually Lyftar's impl session". (Owner, 2026-11-05)

**D21. Lyftar uses Verus.** Parts of Lyftar are proven with Verus. Which parts is not settled here. Cyborg's D66 left this
to Lyftar's design session; the owner chose: "Should we have Verus? yes". (Owner, 2026-11-05)

**D22. What Verus proves.** This settles what D21 left open. Verus proves Lyftar's own checks that stand between safe Rust
and an `unsafe` LLVM call, such as the stale-entry check in erase (D11), index bounds, and matching widths (D3, D4). What
Rust's compiler already enforces needs no proof. What an LLVM call does can only be assumed, so those assumptions are
written down in one place, and `extern` declarations stay outside `verus!` blocks. Claude proposed it; the owner chose it:
"yea". (Owner, 2026-11-05)

**D23. The rules start from Cyborg's.** `IMPLEMENTATION.md` starts from Cyborg's rules, changed for a crate with a C++ shim
and deliberate panics, with Radium's C++ rule added. Where an inherited rule conflicts with a decision here, the decision
wins, so `build.rs` compiles the shim (D16). Nothing undecided is guessed, which replaces the provisional choices in the
draft build prompt. Cyborg's design session advised starting from what exists; Claude proposed this base; the owner chose
it: "yea". (Owner, 2026-11-05)

**D24. Optimizations from the start.** Lyftar is built fast as it goes, never as a plain version sped up later. This means
Lyftar's own overhead on top of LLVM: no runtime check a specific type makes unnecessary, no avoidable allocation on a
build call, and a benchmark tracking what Lyftar adds over raw LLVM calls. Claude proposed carrying over Cyborg's rule; the
owner chose it: "yea". (Owner, 2026-11-05)

**D25. The shim is C++23, built with Clang on macOS.** The shim uses C++23 with libc++ hardening on. GCC can't build it on
this Mac, because a GCC plugin crashed Homebrew's `opt`, most likely from mixing libstdc++ with LLVM's libc++. C++ contracts
therefore wait until Clang has them: a shim function's contract is a comment, an ordinary check at the top of the function,
and a check in its Rust wrapper, which Verus proves (D22). Building LLVM itself with GCC was started as an experiment and
stopped before it built anything, so it is untested. Other platforms are not settled here. Claude proposed Clang and C++23;
the owner chose: "Nah, thought again. Probably too impractical". (Owner, 2026-11-05)

**D26. Three cores per session.** Each Lyftar session uses at most 3 cores at once, for builds, tests and anything it starts.
This replaces the "no limit" rule inherited from Cyborg. The owner asked for "a core limit, like 3"; Claude asked whether
that was the rule for every session; the owner chose: "yea, each". (Owner, 2026-11-05)

**D27. Checks on the C++.** `clang-format` in LLVM's style and `clang-tidy` with warnings as errors are required on the shim.
AddressSanitizer and UBSan run as a separate test run, not in everyday `cargo test`. ThreadSanitizer is left out, since a
context lives on one thread. Getting the sanitizers working is untested; if it goes badly, it comes back to the owner rather
than being dropped. Claude proposed it, from Radium's C++ rule; the owner chose it: "yea". (Owner, 2026-11-05)

**D28. Public at Soclets/Lyftar, pushed automatically.** Lyftar lives in a public repository, `Soclets/Lyftar`. Sessions push
after each commit, running `git pull --rebase` first. Claude suggested a private repository with automatic pushing, as Cyborg
does; the owner chose: "yea, public". (Owner, 2026-11-05)

**D29. The build session.** The session that builds Lyftar is `implementation claude`, in its own clone at
`/Users/ts/Lyftar implementation`, on `main`. This design session stays at `/Users/ts/Lyftar`. Claude proposed it, following
Cyborg's setup; the owner chose it: "yup". (Owner, 2026-11-05)

**D30. `Byte<Bits<8>>`.** LLVM 23's byte type, which holds raw memory in a value and can be any width, is spelled
`Byte<Bits<N>>`, following D7. Lyftar impl asked; Claude proposed it; the owner chose it: "yea". (Owner, 2026-11-05)

**D31. Pointer address spaces.** `Pointer` means address space 0, and is the same type as `Pointer<AddressSpace<0>>`. Any
other space is `Pointer<AddressSpace<N>>`, and a pointer whose space isn't known is `Dynamic`. Lyftar impl asked; Claude
proposed it; the owner chose it: "yea". (Owner, 2026-11-05)

**D32. Specific arrays, vectors and structs.** `[4 x i32]` is `Array<Integer<Bits<32>>, Elements<4>>`, `<4 x i32>` is
`Vector<Integer<Bits<32>>, Elements<4>>`, `<vscale x 4 x i32>` is `ScalableVector<Integer<Bits<32>>, MinimumElements<4>>`,
`{ i32, ptr }` is `Struct<(Integer<Bits<32>>, Pointer)>`, and `<{ i32, i32 }>` is
`PackedStruct<(Integer<Bits<32>>, Integer<Bits<32>>)>`. A named struct stays `Dynamic`, since it can refer to itself and its
body can come later. Lyftar impl asked; Claude proposed it; the owner chose it: "yea". (Owner, 2026-11-05)

**D33. Specific function signatures.** A known signature is `Function<R, (A, B)>`, such as
`Function<Integer<Bits<32>>, (Integer<Bits<32>>, Pointer)>` and `Function<Void, ()>`, so a call's arguments and result are
checked at `cargo build`. Plain `Function` is dynamic. Specific calls go up to a fixed number of arguments, and longer ones
use the dynamic form. A variadic function is `VariadicFunction<R, (A, B)>`, where only the fixed arguments are checked.
Lyftar impl asked; Claude proposed it; the owner chose it: "yea". (Owner, 2026-11-05)

**D34. An exclusive form of replacement.** Where `replace_all_uses_with` would let LLVM delete something a `Value` could still
point at, it panics and names this form: `module.replacement_to_make(old, new)` gives an entry and `module.replace(&[entry])`
takes exclusive access, as erase does (D11), so nothing can be held across it. The everyday `replace_all_uses_with` stays for
the safe case. Lyftar impl found the gap and proposed it; Claude recommended it; the owner chose it: "yea". (Owner, 2026-11-05)

**D35. What is a `Value`, and `BasicBlock`.** Functions, global variables and instructions are each a `Value<T>` with a marker
that keeps LLVM's word, such as `Value<Function>`, `Value<GlobalVariable>`, `Value<Instruction>`, `Value<CallBase>`,
`Value<Terminator>`, `Value<PhiNode<T>>` and `Value<MetadataAsValue>`, since LLVM treats them all as values. A block is its own
type, spelled `BasicBlock`, as LLVM's concepts keep LLVM's words (D5). Lyftar impl proposed the markers and the name `Block`;
Claude proposed `BasicBlock`; the owner chose it: "yea". (Owner, 2026-11-05)

**D36. The names Lyftar impl chose.** Specific calls and structs go up to 12 items. A specific function is added with
`module.add_specific_function::<Function<R, (A, B)>>("name")`. `builder.call` is one name, taking a tuple for a specific function
and a slice for a dynamic one, and a variadic call reads `builder.call(printf, ((format,), &[rest]), "")`. A block as a value is
`Value<BasicBlockAsValue>`. D4's checked cast is `checked_cast`. A constant of a specific aggregate is built dynamically and then
cast. D31's unknown space is plain `Value<Dynamic>`. Lyftar impl chose these; Claude listed them; the owner asked for no changes:
"nah". (Owner, 2026-11-05)

**D37. Aggregate and expression constants belong to a module.** This narrows D11, which said constants belong to the context
and survive. LLVM's own passes, `default<O2>` among them, can delete an aggregate or constant expression once nothing uses it,
as a use-after-free under AddressSanitizer showed in a program with no `unsafe`. So these constants come from a module and
borrow it, as its other values do, and one held across a delete or a pass run does not compile. They can't be shared across
modules. Plain data stays on the context and survives: integers, floats, null, undef, poison, zero, and data arrays and vectors.
Lyftar impl found the hole and preferred a hidden module keeping each constant in use, as did Claude; the owner chose: "the alt".
(Owner, 2026-11-05)

**D38. Bugs are fixed without asking.** A session fixes an actual bug without asking the owner: behaviour that breaks a
decision, a rule, or what the code says it does. A fix that needs a design choice is still that choice, and goes to the owner
under rule 3. Claude asked whether to fix two cases where LLVM ends the process instead of returning an error (D3); the owner
chose: "Bugs shall be fixed without asking, if it's actually a bug". (Owner, 2026-11-05)

**D39. Constants borrow a token from the context.** This replaces D37's borrow of one module, which could not close the hole:
LLVM keeps one copy of an identical constant per context, so a pass on module B could delete a constant held from module A.
Aggregate and expression constants borrow a token from the context, and running passes needs that token exclusively, so a constant
held across a pass on any module does not compile. `Context` stops being a plain copy, and every pass call changes. Because these
constants no longer belong to one module, D37's limit on sharing them may go; that is untested. Plain data stays free of the
token. Lyftar impl found the problem and preferred a run-time panic; Claude proposed the token; the owner chose it: "yea". (Owner,
2026-11-05)

**D40. Verify after each Rust pass.** Lyftar verifies the IR after each Rust pass, and a failure aborts naming the pass (D3), so a
Rust pass can't leave broken IR for the next C++ pass to crash on. This adds to the check before a pipeline, passes and emission
that Lyftar impl already made. The benchmark measures the cost (D24), and if it hurts, it comes back to the owner. Lyftar impl
asked; Claude proposed it; the owner chose it: "yea". (Owner, 2026-11-05)

**D41. Which constants need the token.** `ConstantPtrAuth` and anything built from other values, such as `constant_array`,
`constant_struct` and `constant_vector`, need the token from D39, even when LLVM returns plain data. Raw data such as
`constant_data_array` and `constant_string`, bytes, `token none`, `target none` and inline assembly do not. If one of these turns
out to be deletable, moving it is a bug fix under rule 17. Lyftar impl proposed the list; Claude agreed; the owner chose it:
"yea". (Owner, 2026-11-05)

**D42. Modules live inside the context.** This builds on D39 and changes D11's "a value borrows its module". A constant read back
from a module could otherwise still be deleted by a pass on another module, in safe code. So a module lives inside its context, as
in `context.module(a).get_global(..)`, every value borrows the context, and running passes needs the context exclusively, so no
read can be missed. A module stops being a separately owned value, and an exclusive call on one module blocks values held from any
other. Whether a separate token from D39 is still needed is not settled here. Lyftar impl found the hole and preferred an extra
token argument on each read, as did Claude; the owner chose: "B". (Owner, 2026-11-05)

**D43. How modules live inside the context.** This settles the shape of D42 and replaces D37's module borrow and D39's separate
token. You own an `OwnedModule`, and its contents are only reachable through the context, with `context.module(&a)` to read and
build and `context.exclusive_module(&mut a)` for what needs the whole context, such as passes. Dropping the handle frees the module,
and using one linked away does not compile. `with_context` gives `&mut Context`, which is the only token, so there is no `Constants`
type, `Context` is not a copy, and anything built from values is a method on it. `value.context()` and `type.context()` go away.
Plain data and types borrow nothing and survive passes. Constants can be shared across modules, so D37's limit is gone. A value held
across an exclusive call on another module does not compile, and a tracked handle (D11) is the way around that. Lyftar impl proposed
it after trying it against kayo's passes; Claude agreed; the owner chose it: "yes". (Owner, 2026-11-05)

**D44. No plugins on Windows for now.** `pass_plugin!` is a compile error on Windows, and its plugin tests are off there, because the
official Windows LLVM is built with plugins turned off and a plugin test hung against it. Passes linked into a compiler still work
on Windows. This gets fixed when the owner needs Windows. Lyftar impl proposed it; Claude agreed; the owner chose: "I mean, it's for
me, and I'm on Mac. So, like, we could fix it when I need Windows". Reading that as this option is Claude's. (Owner, 2026-11-06)

**D45. Real garbage collection support.** Lyftar goes beyond naming a strategy: a language built on it can have a precise, moving
collector, with statepoints, LLVM's built-in strategies, custom strategies in Rust, and stack maps it can read while the program
runs. `design/prompts/garbage-collection.md` holds the scope. How the stack map reader is packaged, so that a compiled program need
not link LLVM, is not settled here. A throwaway test language asked whether Lyftar supports GC; Claude suggested waiting for its
choice of memory model; the owner chose otherwise: "Make real GC support". (Owner, 2026-11-07)

**D46. `lyftar-stack-maps`.** The stack map reader is a second crate, `lyftar-stack-maps`, with no LLVM and no shim, because it runs
inside a language's finished program, and that program must not have to link LLVM. This is an exception to D16, which is about not
splitting Lyftar's LLVM API: the reader wraps no LLVM, and only reads bytes. Claude proposed it; the owner chose it: "yea". (Owner,
2026-11-07)

**D47. GC names, and `unsafe` for the operating system.** The garbage collection API keeps the names Lyftar impl took from LLVM's
words, spelled out: `lyftar::garbage_collection` with `register_strategy`, `GarbageCollectionStrategy`, `StatepointCall`,
`StatepointFlags`, `statepoint_pipeline` and `SafepointPlacement`; `Builder` methods such as `garbage_collection_statepoint`,
`garbage_collection_relocate`, `garbage_collection_result` and `garbage_collection_root`; `Value<GarbageCollectionStatepoint>`; and
in `lyftar-stack-maps`, `StackMaps`, `Table`, `Record`, `Statepoint`, `Root`, `Location`, `Roots`, `CallSite`, `parse_section`,
`section_in_file` and `loaded_sections`. `set_garbage_collector` panics on a name nothing registered (D3). Rule 7 now also allows
`unsafe` where `lyftar-stack-maps` asks the operating system for a program's loaded sections, which it can't find otherwise. Claude
proposed the exception and agreed with the names; the owner chose: "okay". (Owner, 2026-11-07)

**D48. `call` takes inline assembly.** `builder.call` accepts inline assembly as it accepts a function, so calling it needs no
`call_indirect` and `to_dynamic`. Claude found the gap while compiling a test for Cyborg's targets; the owner chose: "yea". (Owner,
2026-11-10)
