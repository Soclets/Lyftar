# What Lyftar assumes

Verus proves Lyftar's own checks (`src/checks.rs` and `stack-maps/src/checks.rs`, D22). What an LLVM call does can only be assumed, so those assumptions
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

- **LLVM ends the process over a garbage collection strategy it can't find** (`getGCStrategy`), and its verifier looks one up for
  any `gc.relocate`, even in a function with no collector. So `set_garbage_collector` panics, and `verify`, every pass run, emission,
  both JITs and reading text are errors first (D3, D45). Bitcode isn't checked until one of those.
- **A statepoint with the transition flag only makes code on x86.** AArch64, RISC-V and PowerPC were seen ending the process with
  `Cannot select`, and no target's source mentions the node, so anything but x86 is an error (D45).
- **`erlang`'s table printer writes an ELF section whatever the format.** On Mach-O that corrupted memory, in a run seen to fail one
  time in a few, so anything but ELF is an error (D45).
- **`ocaml`'s table printer ends the process over a frame of 64 KiB or more, or 65536 call sites.** Those can't be seen ahead of
  time without doing the code generator's work, so they aren't caught.

## Garbage collection

- **`place-safepoints` reads a null function without a `gc.safepoint_poll`,** since its check is an `assert`, and Homebrew's LLVM has
  those off. It only rewrites `statepoint-example` and `coreclr`, by name, so `run_passes` makes that an error for those (D45).
- **LLVM's strategy registry has no lock.** `Registry::add_node` writes a plain pointer that a compile on another thread can be
  walking. Registering takes a Rust lock against other registrations, and a release fence before the write. A reader only follows
  that pointer to the node, so this leans on pointer-sized writes being whole and on address dependencies keeping their order, which
  both hold on AArch64 and x86-64. It isn't what C++ promises.
- **A registered strategy lasts for good.** The registry has no way to take one out, so what is registered is leaked on purpose, and
  a plugin registering one must stay loaded, as `opt` keeps them (`PassPlugin::Load` uses a permanent library).
- **A statepoint's stack map record is three constants, deoptimization locations, base and derived pairs, then allocations.** Only
  the allocations are `Direct` (`StackMaps::parseStatepointOpers`, and `lowerStatepointMetaArgs` puts every frame index among them),
  which is how `lyftar-stack-maps` tells them apart, since the record gives no count of pairs. `Statepoints.rst` says the same of
  the rest, and `tests/decision_45_stack_maps.rs` sees it hold.
- **LLVM's optimizer drops a `gc-live` value nothing relocates** (`InstCombine`), and a stack allocation in that list then goes
  too, so a program that reads it afterwards reads a stale pointer. `Statepoints.rst` warns this form is little exercised. So
  `StatepointCall` says to write these after optimizing, and the test runs one only when it doesn't optimize.
- **`rewrite-statepoints-for-gc` adds nothing to a statepoint it finds** (`needsStatepointRewriting`), so inlining a hand-written one
  into a function the pass rewrites leaves what that function holds out of its live list. Seen in the test: a poisoned space
  caught the stale pointer. `StatepointCall` says so.
- **A JIT can't find the `llvm_gc_root_chain` that shadow stack lowering makes up** when it makes each translation unit's code, since
  it isn't in the IR ORC reads. A program defining it itself works: the test's does.
- **LLVM can fold a derived pointer away.** `CodeGenPrepare` rewrites a relocate of a constant offset from its base into arithmetic on
  the relocated base, so such a pointer never reaches the stack map, and that is correct: the object still moves right.
- **A function's stack size in its stack map record, and in an `ocaml` frame table, is what its return moves the stack pointer by**,
  past the return address on x86-64. The test collector finds each caller's stack pointer this way. Where the frame record is at the
  top of the frame, on Darwin and x86-64, it also asserts this against the frame pointer. Linux on AArch64 saves the callee-saved
  registers above the frame record, so there the frame pointer doesn't say where the caller's stack starts.
- **JITLink names the section `__LLVM_STACKMAPS,__llvm_stackmaps` on Mach-O and `.llvm_stackmaps` on ELF, and strips it** with nothing
  referring to it, so a plugin marks it live. **RuntimeDyld loads it,** since Mach-O and ELF treat it as needed to run. Neither is
  in the headers, and `tests/decision_45_moving_collection.rs` sees both hold.
- **A module with an `ocaml` function fails in `Jit` with `Unexpected definitions`,** since its frame table is a symbol the IR doesn't
  have. That is an error at lookup, not a crash. An object made first and given to `add_object_file` works, with RuntimeDyld: JITLink
  also refused such an object on GitHub's Mac, with `Block ... overlaps`, because its labels at section ends make empty blocks, while
  the same code passed on this Mac. That is an error too.
- **`ExecutionEngine` leaves a call to a function it can't find going to address 0 on ELF,** with no error, and macOS finds this
  process's functions through `dlsym` where Linux can't unless they are exported. A program's runtime functions need
  `add_global_mapping`, through `find_declared_function`, since `find_function` only finds bodies.
- **ELF's stack map section is read-only and holds absolute addresses.** So `rust-lld` refuses it in a position-independent program
  without `-z notext`, and the program loads with text relocations. `lyftar-stack-maps` reads it after that, relocated. Seen on
  GitHub's Linux.
- **RuntimeDyld on AArch64 can't reach a call between objects more than 128 MiB apart.** Seen in the collector test as a jump to an
  unmapped address in about 1 run in 13, for `ocaml`'s two objects. `ExecutionEngine` makes code in the large code model by default,
  which has no such limit, and never failed; `use_runtime_dyld` says to do the same.
- **Linkers that strip what nothing refers to drop a stack map section.** rustc does by default, which `-C link-dead-code` turns off.
  Neither `cc` nor `ld` does without `-dead_strip` or `--gc-sections`.

## Self-contained linking

- **lld's library runs again in one process after a link**, even one that failed: `lldMain` said `canRunAgain` every time it was
  tried, and if it ever doesn't, `lld` refuses every later link. **Two links at once crash it** (seen: SIGTRAP), so a lock keeps them
  one at a time.
- **A `libSystem.tbd` listing only the symbols a program uses is enough for `ld64.lld`,** with `dyld_stub_binder` for the lazy
  binding a program for macOS 11 has. What it names has to be in the real `libSystem`, which only shows when the program loads.
- **Linux programs are static and not position-independent,** since a stack map is a read-only section of addresses (D45), which a
  static position-independent program can't relocate.
- **LLVM builds the C APIs of `lto.h` and `Remarks.h` only as shared libraries,** and a static LLVM still installs those, which macOS's
  linker prefers, so `llvm.yml` compiles them as static libraries and leaves the shared ones out.
- **Homebrew's LLVM can't be made static:** its static libraries want a shared `libz3` with no static copy, and its lld is shared only.
- **compiler-rt's builtins have C++ in them on AArch64,** which CMake builds with the runner's own compiler unless told otherwise.

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
