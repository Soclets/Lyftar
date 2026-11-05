This is my own prompt, and it is my go: build Lyftar now.

<context>
Lyftar is a Rust crate that does what inkwell and llvm-plugin do, for the latest stable LLVM: building LLVM IR from safe Rust, and writing LLVM passes in Rust. I am building it for my own programming languages, which today use inkwell on LLVM 21 and 22 and C++ for custom passes. inkwell stops at LLVM 22 and llvm-plugin stops at LLVM 18, so they can't move to LLVM 23 on them.

You are implementation claude, working in your own clone at "/Users/ts/Lyftar implementation", branch main. Clone it from https://github.com/Soclets/Lyftar.git if it isn't there, and set its git author as CLAUDE.md gives it. The repository is public. The design session, design claude, stays in /Users/ts/Lyftar. This is macOS on ARM. LLVM 23.1.2 is at /opt/homebrew/opt/llvm and is not on PATH (llvm-config is in its bin folder, libclang in its lib folder).

Read these first, in full: DECISIONS.md, IMPLEMENTATION.md and CLAUDE.md. The decisions bind this work, and D19 replaces D18. IMPLEMENTATION.md's rules come first wherever this prompt seems to differ.

Uncommitted in /Users/ts/Lyftar, yours to copy into your clone, fix and commit: Cargo.toml, .gitignore, build.rs, shim/shim.cpp, tools/bindings/. Don't change them there. Generating the bindings failed with UnsupportedEdition(Edition2024, RustTarget(Stable(82, 0))), because bindgen needs its Rust target set to one that supports edition 2024. Set the crate version to 0.1.0 (D19). build.rs passes llvm-config's -std=c++17, and D25 needs C++23 with libc++ hardening instead.
</context>

<consequences_of_the_decisions>
Earlier small tests used fake handles with no LLVM behind them, so none of this is proven yet. Each point below follows from the decisions and is easy to get wrong:
- D4, D10 and D11 together mean a module's values carry both a context lifetime and a module borrow, something like Value<'ctx, 'module, T>. The context lifetime is invariant. The module lifetime is covariant so that constants, which belong to the context, can be used with any module.
- That covariance lets two modules in one context unify at compile time, so the builder needs a runtime check that a value, block or function belongs to its module, and panics otherwise (D3).
- The list handed to erase (D11) has no lifetime, so a stale or duplicate entry would be a use-after-free in safe code. Entries must carry something that lets erase detect this and panic, such as a generation count on the module that every exclusive operation bumps.
- Emitting an object file runs codegen passes that can change the module, so it needs exclusive access, as running passes does.
- LLVM here is built with -fno-exceptions, so a Rust panic must not unwind through C++ frames. Catch it at the shim boundary and abort with its message (D3).
- LLVMInitializeNative* and the macro-generated target initializers are not in the generated bindings. They go through the shim.
- A plugin built with GCC against Homebrew's libc++ LLVM crashes opt, so use Clang on macOS (D25). Clang has no C++ contracts yet.
- Verus pins its own Rust toolchain, 1.99.1 when last checked, while the default stable here is 1.96.1. Verus only checks code inside verus! blocks. A normal Cargo build reportedly strips the proofs, but it's untested whether a project using Lyftar then needs Verus's crates. Find that out early, because it affects all my languages, and tell design claude what you find.
- In LLVM 23 the branch opcode is split into UncondBr and CondBr, PassPlugin.h is in llvm/Plugins/, and the plugin API is version 2. Work from the installed headers, not from memory of older LLVM.
</consequences_of_the_decisions>

<task>
Build all of Lyftar, as one crate (D16):
- Raw bindings: generated, committed for macOS, generated during the build on other platforms (D12 to D15), public as lyftar::raw.
- Safe API: context, modules, every type kind, values, constants, functions, blocks, globals, metadata, attributes and the complete instruction builder, with deletion and tracked handles per D11. Then the modules following the C API: targets and object emission, object files, bitcode and IR reading and writing, debug info, the JIT (execution engine, ORC, LLJIT), LTO, linking, disassembly and remarks.
- Passes in Rust, with no C++ for the pass author: module and function passes, pipelines and LLVM's extension points, the analyses a real pass needs (at least dominator trees and loop info), and CloneFunction and InlineFunction, usable both linked into a compiler and loaded as a plugin by opt or clang.
</task>

<done_when>
The work is done when all of these hold:
1. A clean cargo build and cargo test pass on this Mac against the real LLVM 23.1.2.
2. A test builds a program with Lyftar, optimizes it, emits an object file, links it, runs it, and checks its output. Another test runs generated code through the JIT.
3. Every safety promise has a test against real LLVM: compile-fail tests for mismatched specific types, values escaping or crossing contexts, and values held across a delete or a pass run; should-panic tests for each runtime check, asserting the message. Each compile-fail test has a near-identical passing twin, so it can't pass for the wrong reason.
4. kayo's two passes, /Users/ts/CyborgPL/kayo/llvm-passes/heap_to_stack.cpp and recursive_inline.cpp, are rewritten in Rust as examples in this repo, and behave as the C++ versions do, using kayo's pass tests as the reference.
5. A Rust pass runs both linked into a compiler and through opt -load-pass-plugin.
6. API coverage: coverage.json lists every function in the raw bindings as wrapped, or as deliberately not wrapped with a reason, and none is left unclassified. Also, every inkwell API used by these projects has a Lyftar equivalent, recorded the same way: CyborgPL/kayo/compiler, QuiPaws/crates/quipaw-codegen, Verb-AL, Luarust/crates/luarust-jit, "Advanced High-Performance Calculations Language (AHPCL)"/crates/ahpcl-codegen and RustroverProjects/QuickMathematicalCalculationsLanguage, all under /Users/ts.
7. Verus proves the checks in D22, and the LLVM assumptions are written down in one place.
8. clang-format and clang-tidy pass on the shim, and the AddressSanitizer and UBSan test run from D27 passes.
9. A benchmark tracks what Lyftar adds over raw LLVM calls (D24), and the tracing check from IMPLEMENTATION.md runs.
10. The public API has doc comments, and a short README covers what Lyftar is, how to use it, and the command that regenerates the bindings.
</done_when>

<boundaries>
- Commit and push at each working step, as CLAUDE.md says. Publishing to crates.io, editing any other repository, and running brew upgrade need my yes first. The other repositories are read-only references. Installing Verus and its toolchain is fine, as long as the default Rust toolchain stays as it is.
- Linux and Windows can't be run here. Write the build-time bindings path and a CI workflow for them, and report them as untested.
- Implement real logic that works for all valid inputs. If a test seems wrong, tell me instead of working around it. Never remove or weaken a test to get a pass.
- Keep it no bigger than the task needs: no feature flags (D2, D15, D16), and no abstractions or extras nobody asked for.
- Read documentation and LLVM headers yourself, and open any code before making claims about it. Use subagents only for large, independent, parallel pieces of the build, never to summarize something for you, read docs for you, or check your work, and don't choose a smaller model for them.
</boundaries>

<design_choices>
Design choices are mine, and nothing undecided is guessed (rule 3). Applying a rule that's already decided isn't guessing: naming a method from LLVM's name under D5 and D8 needs no question. Internal details no user can see are yours. When something a user of Lyftar could see isn't covered, such as how Byte<Bits<N>> or pointer address spaces are spelled, leave that part unbuilt, note it in PROGRESS.md, and message design claude (the session named Lyftar) with the question, then carry on with whatever doesn't depend on it. If a decision turns out to be impossible or unsound against real LLVM, say so in a sentence and stop building on it until it's answered.
</design_choices>

<long_running_work>
This is a very long task, so plan it clearly and work in increments, starting with one thin slice through real LLVM before widening it. Keep state where a fresh context can find it: PROGRESS.md for freeform notes, coverage.json for coverage, and git for history. Your context is compacted automatically as it fills, so don't wrap up early because of its size, and don't leave significant work uncommitted.

A message with no tool call ends your turn and stops the work. I don't want the turn to end on a summary that announces the next step, an offer to continue, a list of decisions that don't block anything, or a report just because a milestone is done. Put status notes in the same message as your next tool call and carry on. The stops I do want are when done_when is fully met, when nothing can move without me, and when an action needs my yes under the boundaries.
</long_running_work>

<communication>
I read these in chat and I get lost in dense text. Before the first tool call, say in one sentence what you're starting. While working, give a brief plain update when you find something important or change direction. When you finish, lead with what now works, then list what is untested and which questions are still open, and keep it short.
</communication>

Build Lyftar until done_when is met.
