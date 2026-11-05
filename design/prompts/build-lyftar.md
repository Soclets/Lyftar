This is my own prompt, and it is my go: build Lyftar now.

<context>
Lyftar is a Rust crate that does what inkwell and llvm-plugin do, for the latest stable LLVM: building LLVM IR from safe Rust, and writing LLVM passes in Rust. I am building it for my own programming languages, which today use inkwell on LLVM 21 and 22 and C++ for custom passes. inkwell stops at LLVM 22 and llvm-plugin stops at LLVM 18, so they can't move to LLVM 23 on them.

Repository: /Users/ts/Lyftar, branch main, local only. Rust 1.96.1 on macOS ARM. LLVM 23.1.2 is at /opt/homebrew/opt/llvm and is not on PATH (llvm-config is in its bin folder, libclang in its lib folder).

Read these first, in full: DECISIONS.md and CLAUDE.md. D1 to D19 are my decisions and they bind this work; D19 replaces D18. CLAUDE.md has the commit style, and every commit follows it.

Already on disk and uncommitted, yours to keep and fix: Cargo.toml, .gitignore, build.rs, shim/shim.cpp, tools/bindings/. Generating the bindings failed with UnsupportedEdition(Edition2024, RustTarget(Stable(82, 0))), because bindgen needs its Rust target set to one that supports edition 2024. Set the crate version to 0.1.0 (D19).
</context>

<consequences_of_the_decisions>
Earlier small tests used fake handles with no LLVM behind them, so none of this is proven yet. Each point below follows from the decisions and is easy to get wrong:
- D4, D10 and D11 together mean a module's values carry both a context lifetime and a module borrow, something like Value<'ctx, 'module, T>. The context lifetime is invariant. The module lifetime is covariant so that constants, which belong to the context, can be used with any module.
- That covariance lets two modules in one context unify at compile time, so the builder needs a runtime check that a value, block or function belongs to its module, and panics otherwise (D3).
- The list handed to erase (D11) has no lifetime, so a stale or duplicate entry would be a use-after-free in safe code. Entries must carry something that lets erase detect this and panic, such as a generation count on the module that every exclusive operation bumps.
- Emitting an object file runs codegen passes that can change the module, so it needs exclusive access, as running passes does.
- LLVM here is built with -fno-exceptions, so a Rust panic must not unwind through C++ frames. Catch it at the shim boundary and abort with its message (D3).
- LLVMInitializeNative* and the macro-generated target initializers are not in the generated bindings. They go through the shim.
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
7. The public API has doc comments, and a short README covers what Lyftar is, how to use it, and the command that regenerates the bindings.
</done_when>

<boundaries>
- Commit locally, in the house style, at each working step. Pushing, creating the GitHub repository, publishing to crates.io, editing any other repository, and running brew upgrade all need my yes first. The other repositories are read-only references.
- Linux and Windows can't be run here. Write the build-time bindings path and a CI workflow for them, and report them as untested.
- Implement real logic that works for all valid inputs. If a test seems wrong, tell me instead of working around it. Never remove or weaken a test to get a pass.
- Keep it no bigger than the task needs: no feature flags (D2, D15, D16), and no abstractions or extras nobody asked for.
- Read documentation and LLVM headers yourself, and open any code before making claims about it. Use subagents only for large, independent, parallel pieces of the build, never to summarize something for you, read docs for you, or check your work, and don't choose a smaller model for them.
</boundaries>

<design_choices>
Design choices are mine. Apply the existing rules yourself for routine calls, such as naming a method under D5 and D8. When the decisions don't cover something and it would shape the public API, such as how Byte<Bits<N>> or pointer address spaces are spelled, make the most reversible choice that lets work continue, and add it with your recommendation to OPEN_QUESTIONS.md. Those stay provisional: only choices I've made go in DECISIONS.md. If a decision turns out to be impossible or unsound against real LLVM, say so in a sentence, stop building on it, and ask me, along with any question that truly blocks all remaining work. Ask in plain chat, one question at a time, in simple language, with no popups or answer buttons.
</design_choices>

<long_running_work>
This is a very long task, so plan it clearly and work in increments, starting with one thin slice through real LLVM before widening it. Keep state where a fresh context can find it: PROGRESS.md for freeform notes, coverage.json for coverage, and git for history. Your context is compacted automatically as it fills, so don't wrap up early because of its size, and don't leave significant work uncommitted.

A message with no tool call ends your turn and stops the work. I don't want the turn to end on a summary that announces the next step, an offer to continue, a list of decisions that don't block anything, or a report just because a milestone is done. Put status notes in the same message as your next tool call and carry on. The stops I do want are when done_when is fully met, when nothing can move without me, and when an action needs my yes under the boundaries.
</long_running_work>

<communication>
I read these in chat and I get lost in dense text. Before the first tool call, say in one sentence what you're starting. While working, give a brief plain update when you find something important or change direction. When you finish, lead with what now works, then list what is untested and what's in OPEN_QUESTIONS.md, and keep it short.
</communication>

Build Lyftar until done_when is met.
