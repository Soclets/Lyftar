This is my own prompt, and it is my go: make Lyftar self-contained, as Zig is (D49). Start by finding out and proposing, as the boundaries below say, before building anything a user could see.

<context>
You are implementation claude, in "/Users/ts/Lyftar implementation", on main. Pull first, and read DECISIONS.md and IMPLEMENTATION.md in full, D49 especially. Recording D48 and D49 made the tracing test fail until something traces them.

The goal: a compiler built on Lyftar, such as Cyborg's, can build and link for every supported target with nothing else installed. That means no LLVM, no outside linker and no system SDK, the way a Zig release works. Cyborg's first targets are macOS on Apple Silicon, Linux on ARM, and bare-metal ARM Cortex-M4 with a linker script. Until this is done, Cyborg calls the installed `ld.lld`. A front end's own startup code and linker scripts stay its job.

What's known today:
- kayo already embeds lld through its library: /Users/ts/CyborgPL/kayo/compiler/lld/shim.cpp and that crate's build.rs. Its notes say lld's library isn't made to run twice in one process.
- Homebrew's LLVM 23.1.2 ships static libraries (213 `libLLVM*.a`, 292 MB), and `llvm-config --link-static` works, but its system libraries still pull in shared `libz3`, `zstd` and `libxml2`.
- Homebrew's lld 23.1.0 ships only shared libraries, and its version differs slightly from LLVM's.
- Homebrew's Zig 0.16 links shared LLVM and lld 21 itself, so it isn't a model of static linking. It does show how Zig ships system files: lib/zig/libc holds darwin (libSystem.tbd and SDKSettings.json), glibc, musl and others, 225 MB in all.
</context>

<task>
1. Find out, prototyping in scratch, what each part takes on this Mac:
   - getting static LLVM and lld that match exactly
   - linking them into a Rust program with no shared libraries beyond the operating system's own
   - linking in-process for Mach-O, ELF on Linux, and bare-metal ELF with a linker script
   - linking more than once in one process
   - linking a normal macOS and Linux program with no SDK or libc installed
2. Send design claude one short proposal covering the choices only Tankun can make, each with your recommendation. These include:
   - where Lyftar's static LLVM and lld come from: Homebrew, a source build, or prebuilt files Lyftar publishes
   - which system files Lyftar ships, and whether their licences allow it
   - the public linking API
   - how big a compiler gets
   - what D2's per-release upgrades cost now
3. Build it once those are decided.
</task>

<done_when>
1. A test compiler built on Lyftar links a hello-world program for macOS on Apple Silicon and ELF Linux on ARM, plus a bare-metal Cortex-M4 image from a linker script, in-process, with no LLVM, lld, Xcode SDK or libc installed or on any path it searches. It shows that by running somewhere they really aren't present, such as a clean user or a container.
2. The macOS program runs here, and the Linux one runs in a container or VM if one is available. The Cortex-M4 image passes `llvm-objdump` and `llvm-readobj` checks on its entry point and sections. If you can't run the Linux program, say so instead of claiming it.
3. Linking twice in one process works, or is refused with a clear error, as decided.
4. CI passes, the README says how to build a self-contained compiler, and ASSUMPTIONS.md and coverage.json are current.
</done_when>

<boundaries>
- Stay within IMPLEMENTATION.md: 3 cores, stop at 8:30 PM, and no test removed or weakened. A source build of LLVM on 3 cores takes hours, so run it in the background and keep working on what doesn't need it.
- Anything a user of Lyftar could see that isn't decided waits for Tankun through design claude, the session named Lyftar. Prototypes in scratch are fine, and internal details aren't decisions.
- Installing tools such as cmake, ninja or Homebrew's lld is fine under rule 15. Changing or upgrading Homebrew's LLVM, publishing release files, and editing other repositories need Tankun's yes first.
- If something here can't be done, such as a licence that forbids shipping a file, report it instead of working around it. Fix actual bugs as you find them (rule 17).
- Do the reading yourself. Use subagents only for large independent pieces, never to summarize, read docs or check your work, and don't choose a smaller model for them.
</boundaries>

<long_running_work>
Keep notes in PROGRESS.md, and commit and push at each working step. A message with no tool call ends your turn and stops the work. I don't want the turn to end on a summary announcing the next step, an offer to continue, a list of decisions that don't block anything, or a report just because a step is done. The stops I do want are when the proposal is sent and nothing else can move without my answers, when done_when is met, and at 8:30 PM.
</long_running_work>

<communication>
I get lost in dense text, so say things plainly. Give a brief update when you find something important or change direction. When you finish, lead with what now works, then what is untested and which questions are still open, and keep it short.
</communication>

Find out, propose, and once decided, build Lyftar's self-contained linking until done_when is met.
