# What Lyftar assumes

Verus proves Lyftar's own checks (`src/checks.rs`, D22). What an LLVM call does can only be assumed, so those assumptions
are here, in one place. Each comes from LLVM 23.1.2's headers, or was seen against the real library where a test says so.
If one turns out false, the safety promise named beside it falls with it.

## Types and identity

- **LLVM uniques types in a context.** Two values have the same type exactly when their type pointers are equal, so
  `same_identity` on pointers is enough for matching widths and types (D3, D4). `Type.h` says so.
- **LLVM uniques literal structs, arrays, vectors and function types as well**, so a specific marker can be compared by
  pointer with the type LLVM gives for it, and a named struct is never that pointer (D32, D33).
- **Types and metadata strings last as long as their context** (D10, D11).
- **A value's kind doesn't change.** A checked cast stays true for as long as the value lives (D4).

## What deletes what

- **`ConstantData` has no use list** (`Value::hasUseList`), and is never deleted while its context lives (D11).
- **Passes can delete an aggregate or constant expression that belongs only to the context.** `GlobalDCEPass` and
  `EliminateAvailableExternallyPass` destroy the initializer of a global they empty, when nothing else uses it, and the default
  pipelines run them. LLVM keeps one copy of an identical constant per context, so that can be a constant another module's code
  holds. Hence D43: every value borrows the context, and anything that can delete needs it exclusively.
- **Only what `'static` covers survives passes.** `Value<'context, 'static, _>` is only ever plain data (`ConstantData`) or inline
  assembly, which `tests/decision_43_modules.rs` checks against the source. `isSafeToDestroyConstant` refuses `ConstantData`, and no
  named pass deleted any in a probe of 271 pipelines. Inline assembly has no direct caller of its `destroyConstant`.
- **Nothing but passes, code generation and exclusive calls deletes these.** Dropping or cloning a module, linking, stripping debug
  information, erasing globals and a bitcode round trip deleted none in the same probe, and neither did reading.
- **An `ExecutionEngine` only shares the context, though it can compile.** It compiles for this machine, and the backends found
  calling `destroyConstant` directly are XCore and SPIR-V, which aren't hosts. A probe with MCJIT deleted nothing.
- **A shared call to LLVM deletes nothing a `Value` can point at**, except the two cases `replace_all_uses_with` refuses:
  - It can destroy a constant other than a global that uses the value being replaced. Seen against the real library: it
    changes the constant in place, unless what the constant would become exists already.
  - LLVM moves that metadata in place only when the replacement has no metadata of its own, and the two are both constants
    or local to one function. In every other case it deletes the metadata. `ValueAsMetadata::handleRAUW` isn't in the
    headers; `tests/decision_11_deletion.rs` sees the in-place case hold.
- **`Module::replace` holds exclusive access, so LLVM can delete those.** What it can't cover is a constant in its own list,
  which LLVM could delete midway, so `lyftar_replace_find_conflict` refuses that list. A constant that uses a module's value
  has the module's lifetime, so nothing with `'static` can be among what goes.
- **A uniqued metadata node is deleted only when it is unresolved** and an operand changes, which
  `lyftar_metadata_node_can_change_operand` checks.
- **Erasing a block.** A block address is rewritten by LLVM when its block goes, and `removeDeadConstantUsers` clears only
  constants nobody uses.
- **`WeakVH` becomes null when its value is deleted, and doesn't follow `replaceAllUsesWith`.** This is what a tracked
  handle rests on. `ValueHandle.h` says so.

## Generations

- **A module's generation is unique across all modules** because it comes from one atomic counter that only goes up, and a
  64-bit counter doesn't wrap in any real run. `entry_is_current` is proven to compare them; what that comparison means
  rests on every exclusive operation changing the generation, which is Lyftar's own code and not an LLVM assumption.

## Rust's standard library

- **`slice::sort_unstable` sorts.** `sorted_has_no_duplicates` is proven to find neighbours that match, and a lemma shows a
  sorted list whose neighbours differ has no duplicates. Sorting itself is assumed.

## Global state in LLVM

- **LLVM's target registry takes no lock, and neither does `libLTO`'s setup.** Seen against the real library: two threads
  registering at once tied the list of targets into a loop, and `TargetRegistry::lookupTarget` spun for good.
  `tests/decision_16_threads.rs` reproduces that against code without the fix. So `set_up_targets` registers every target
  once, `libLTO`'s turn included, before anything can read the registry, and nothing writes to it again.
- **`libLTO`'s global context, last error message and options are unguarded**, which `lto.h` implies by saying only
  `lto_module_create_in_local_context` is thread-safe. Lyftar holds one lock around every `libLTO` call.
- **LLVM has no other global state that safe Lyftar calls change.** The calls that set LLVM's command-line options do change
  some, and are `unsafe` for that reason.

## What LLVM needs from a module

- **LLVM's passes and code generators assume a valid module**, and crash on one that isn't: seen against the real library, a
  block with no terminator segfaults both `default<O2>` and object emission. So they are given only a module the verifier
  passes, which also closes `strip-debug-declare` freeing plain data, since that took a malformed `llvm.dbg.declare`. Each Rust
  pass is verified after it runs too (D40). A function pass is verified only for its own function, the only one it can change.

## What ends the process

- **LLVM's `report_fatal_error` can't be caught.** Its handler must not return, and a panic can't cross C++. So Lyftar turns into
  errors only the cases seen against the real library: running every pass `opt --print-passes` names, without the sanitizer,
  printing and similar ones, found `default<Os>` and `<Oz>` and their LTO forms, `function-import` with no summary file, and
  `free-machine-function`, which frees an invalid pointer. A pipeline's parameters weren't all tried. Others may exist.
- **A backend's fatal errors can't be caught.** A module the verifier passes doesn't mean a target can emit it. A `ptrauth`
  constant anywhere but AArch64 is turned into an error, since 15 targets were seen ending the process over it. SPIR-V ends the
  process on some types, which isn't caught, and neither would anything else a backend refuses that way.
- **Bitcode with debug information that fails the verifier still ends the process.** LLVM's reader upgrades debug information
  as it reads, and has no switch for that. Text is checked first.

## Pass runs

- **`StructuralHash` plus counts of a module's contents catch a pass that lied about `PreservedAnalyses::All`.** A collision
  would miss a lie. It can't make anything unsafe, because analyses are cleared on the generation, not on this check.
- **LLVM's dominator tree gives null for a block it doesn't know.** `DominatorTree::getNode` says so.
- **LLVM's pipeline parser may call a pass's maker more than once.** This was seen against the real library, and the docs
  say so.

## Lifetimes LLVM expects

- **`LLVMPassBuilderOptionsSetAAPipeline` keeps the pointer it is given.** Its header says so. The wrapper keeps the text.
- **`LTOModule` keeps a reference to the bytes it read** (`MBRef` in `LTOModule.h`). The wrapper keeps a copy.
- **`thinlto_codegen_add_module` keeps the bytes for the generator's life.** Its header says so. They are `'static`.
- **`libLTO` merges modules only within one context**, as `lto.h` says of its code generators. So a generator reads its own
  modules into its own context.
- **A `libLTO` generator in the global context leaves its diagnostic handler on that context when it goes.** Seen against the
  real library: the next error there calls through it and crashes. So Lyftar doesn't wrap `lto_codegen_create`.
- **`getTerminator` asserts on a block with no terminator**, so Lyftar asks `hasTerminator` first.
