This is my own prompt, and it is my go: build real garbage collection support into Lyftar. Stack map packaging is the one exception, explained below.

<context>
You are implementation claude, in "/Users/ts/Lyftar implementation", on main. Pull first, and read DECISIONS.md and IMPLEMENTATION.md in full. D45 is this work. Rules 3, 4, 11, 13 and 17 matter most here. Recording D45 made the tracing test fail until something traces it, so trace it as you start.

I want my languages to be able to have a precise, moving collector on Lyftar. Today Lyftar only has `set_garbage_collector` and the raw pieces, and nothing has tested them. A precise collector has to know exactly which stack slots hold pointers, so it can move objects and rewrite those slots.

Read /Users/ts/QuiPaws/docs/statepoints.md and /Users/ts/QuiPaws/spikes/spike-statepoints/src/main.rs first. That experiment, on inkwell and LLVM 22 with no shim, got `statepoint-example`, `rewrite-statepoints-for-gc` by name, and a stack map parsed by about 70 lines of Rust that matched `llvm-readobj --stackmap`. It never linked or ran the result, and it hasn't been run on Lyftar or LLVM 23. Its write-up also records lessons to carry over: `rewrite-statepoints-for-gc` has to run after inlining; GC handles must be pointers in the strategy's address space, because an integer never reaches the stack map; pointers come as base and derived pairs; and a stack map only covers the compiled program's stack, so a runtime's own temporaries need roots of their own.

LLVM 23's headers have IR/Statepoint.h, IR/GCStrategy.h, IR/BuiltinGCs.h, CodeGen/GCMetadataPrinter.h, Object/StackMapParser.h, and the gc.statepoint, gc.relocate, gc.result, gcroot, gcread and gcwrite intrinsics. opt lists rewrite-statepoints-for-gc, place-safepoints, shadow-stack-gc-lowering and gc-lowering. Work from those headers.
</context>

<task>
Build these, in this order, each one working through real LLVM before the next:
1. The built-in strategies. Prove `statepoint-example` works on Lyftar with `Pointer<AddressSpace<1>>` as the GC pointer, then cover `coreclr`, `shadow-stack`, `erlang` and `ocaml`, including the gcroot, gcread and gcwrite intrinsics that shadow-stack needs.
2. Statepoints written directly as well as through the pass: gc.statepoint calls with gc-live, deopt and gc-transition bundles, gc.relocate and gc.result, safe under D3 and D43, plus place-safepoints. Make the correct pass ordering easy to get right.
3. A stack map reader: parse the section from an object file and from a loaded or linked program on macOS and Linux, merge several translation units, and look up a return address to get its roots, as base and derived pairs with their locations. Looking one up must not allocate (D24), because it runs during a collection, inside my languages' finished programs. Malformed bytes return an error (D3), and its bounds checks are proven under D22.
4. The JIT path: find stack maps for code compiled by `Jit` and `ExecutionEngine`.
5. Custom strategies in Rust, with no C++ for the author, through the shim: a Rust-defined GCStrategy that works linked in, and as a plugin where plugins work (D44).
</task>

<stack_map_packaging>
A compiled program needs the reader while it runs, but it must not have to link libLLVM. D16 makes Lyftar one crate, which always compiles the shim and links LLVM, so the reader can't simply live in it as things stand. This packaging is my decision. design claude is taking it to me, so check DECISIONS.md for the answer before step 3. Until it's there, build everything else, keep the parser free of any LLVM dependency, and leave its public home unbuilt.
</stack_map_packaging>

<done_when>
1. A test compiles a program with Lyftar that allocates through a small moving collector written for the test, forces collections while live objects are held across calls, and prints results that prove the objects really moved and every root was rewritten. It runs linked on macOS and Linux, and through the JIT, cleanly under the AddressSanitizer run. Run it with optimization on as well as off, and for each strategy that can move objects.
2. The parser agrees with `llvm-readobj --stackmap` on real objects, including several translation units and derived pointers that differ from their bases.
3. A test finds the stack map in a linked program that doesn't link LLVM, once packaging is decided.
4. A Rust-defined custom strategy compiles and runs through the same collector test.
5. The benchmark tracks stack map lookups and shows no allocation. CI passes, and coverage.json and ASSUMPTIONS.md are current.
Windows is out of scope beyond keeping CI green (D44).
</done_when>

<boundaries>
- Stay within the rules in IMPLEMENTATION.md: 3 cores per session, stop at 8:30 PM, and no test removed or weakened to get a pass.
- Anything a user of Lyftar could see that isn't decided waits, and the question goes to design claude, the session named Lyftar. Naming from LLVM's words under D5 and D8 isn't a question. Carry on with what doesn't depend on the answer.
- Other repositories are read-only, including QuiPaws and the test language. The test language may send you findings through design claude.
- If a decision turns out to be impossible or unsound with GC, say so and stop building on it. Fix actual bugs as you find them (rule 17).
- Do the reading yourself. Use subagents only for large independent pieces, never to summarize, read docs or check your work, and don't choose a smaller model for them.
</boundaries>

<long_running_work>
Keep notes in PROGRESS.md, and commit and push at each working step. A message with no tool call ends your turn and stops the work. I don't want the turn to end on a summary announcing the next step, an offer to continue, a list of decisions that don't block anything, or a report just because a step is done. The stops I do want are when done_when is met, when nothing can move without me, and at 8:30 PM.
</long_running_work>

<communication>
I get lost in dense text, so say things plainly. Give a brief update when you find something important or change direction. When you finish, lead with what now works, then what is untested and which questions are still open, and keep it short.
</communication>

Build real garbage collection support until done_when is met, apart from what waits on the packaging decision.
