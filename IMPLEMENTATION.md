# Lyftar — Implementation rules

The rules for the code that implements Lyftar. `DECISIONS.md` records each decision with the owner's words; this file
collects the rules. A hook prints it into every session at startup and after compaction, so keep it short. When a rule
changes, `DECISIONS.md` gets a new entry and this file is updated to match. Nothing gets built until the owner says go.

## Rules

1. **Language.** Rust, plus a C++ shim for what LLVM's C API can't reach, custom pass support included. `build.rs` compiles
   the shim (D16). Tools are Rust too, in `tools/`. The exceptions are one-line commands in config and scratch work that is
   never committed.
2. **Where.** Only design claude edits `DECISIONS.md` and `IMPLEMENTATION.md`.
3. **No design in code.** Nothing undecided is guessed. Anything a user of Lyftar could see waits, and the question goes to
   the owner through design claude. Internal details no user can see aren't decisions, so the session just makes them.
4. **Tests.** Tests run against real LLVM, and what they build goes through LLVM's module verifier. Nothing is committed
   while a test fails.
5. **Panics and errors.** A bug in the compiler using Lyftar panics at the call and names the problem (D3). What can really
   fail at run time returns an error. No `unwrap`, `expect` or panic anywhere else.
6. **Style.** `rustfmt`, `clippy`, and comments that say why. `#![warn(clippy::pedantic)]` and
   `#![deny(clippy::unwrap_used, clippy::expect_used)]`. `clippy::panic` is denied too, and allowed at each D3 check.
7. **`unsafe`.** Only where Rust calls LLVM or the shim, each block saying why it is sound. Everything else is safe code.
8. **Verus.** Lyftar's own checks before an `unsafe` LLVM call are proven. What LLVM does is assumed, and those assumptions
   are written down in one place. `extern` declarations stay outside `verus!` blocks. (D21, D22)
9. **C++.** C++23 with libc++ hardening on, built with Clang on macOS (D25). Small, and only behind `extern "C"` functions, each
   with a written contract: who owns each pointer, how long it lives, and which values are allowed. The function checks its
   contract at the top, and Rust wraps each one once, in a safe function that checks it too. A Rust panic
   never crosses into C++. Banned: raw `new` and `delete`, C-style casts, `reinterpret_cast`, unions, pointer arithmetic,
   uninitialised variables, `goto` and variadic functions, except where LLVM's own API requires one, with a comment saying so.
10. **Commits.** `kind.folder.narrowing: sentence (owner: "their words"; D1)`. Kinds: `decision`, `design`, `feature`, `fix`,
    `test`, `house`. Each commit ends with its session's trailer, such as
    `Co-Authored-By: design claude <noreply@anthropic.com>`.
11. **Cores.** Each session uses at most 3 at once, for builds, tests and anything it starts. (D26)
12. **Prompting.** Before prompting a subagent or a new session, a session reads the current prompting guide for that AI, in
    full and itself, and again after every compaction. Messaging another session isn't prompting, and isn't written like
    prompting: write it like one colleague talking to another.
13. **Stop at 8:30 PM.** Every session stops then, and suggests the owner stops too.
14. **Scratchpad.** Scratch space is for what can be lost. Anything needed later goes in the repository and gets committed.
15. **Downloads are free.** A dependency or tool is chosen on maturity, safety, maintenance and speed. A less mature or less
    safe option never wins because the better one needs installing. If an install needs the owner's approval, ask for it.
16. **Optimizations from the start.** Lyftar is built fast as it goes, never as a plain version sped up later: no runtime check
    a specific type makes unnecessary, no avoidable allocation on a build call, and a benchmark tracking what Lyftar adds over
    raw LLVM calls. (D24)

## Tracing

Code that carries out a decision says so with a comment, in Rust and C++ alike:

```rust
// trace: D3
```

A check fails on a tag that names no decision, and lists the decisions nothing traces yet.

## Naming

- **Full words, no exceptions.** `get_element_pointer`, not `gep` (D5).
- **Every number says its unit.** `size_in_bits`, `alignment_in_bytes` (D8).
- **One name per concept**, and LLVM's concepts keep LLVM's words, spelled out.
- **Rust's casing**, which `clippy` enforces. C++ follows LLVM's style.
- **Tests are named after their decision.**

## Not decided yet

- Whether C++ runs under sanitizers, and `clang-tidy` and `clang-format`.
- The build session's name, whether it works in its own clone, and when anything is pushed.
