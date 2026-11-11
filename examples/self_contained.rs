//! A small compiler built on Lyftar, for D49's test: it compiles a hello world for macOS on Apple Silicon and for Linux on ARM and
//! x86-64, and a Cortex-M4 image, compiles a C file for each with `lyftar::compile_c` (D55), and links each with `lyftar::link`,
//! with nothing else installed. `self_contained <folder>` writes `hello-macos`, `hello-linux-aarch64`, `hello-linux-x86_64`,
//! `cortex-m4.elf` and the linker script it used into the folder.

use std::path::{Path, PathBuf};

use lyftar::compiling::CCompileOptions;
use lyftar::linking::SystemTarget;
use lyftar::target::{CodeGenerationFileType, CodeGenerationOptimizationLevel, CodeModel, RelocationMode, Target};
use lyftar::{Bits, Builder, Integer, compile_c, link, with_context};

type I32 = Integer<Bits<32>>;

/// The objects and links, for each target: its triple, CPU, where the program goes, and the target to link for.
const HELLOS: [(&str, &str, &str, SystemTarget); 3] = [
    ("arm64-apple-macosx11.0.0", "apple-m1", "hello-macos", SystemTarget::MacOsAarch64),
    ("aarch64-unknown-linux-musl", "generic", "hello-linux-aarch64", SystemTarget::LinuxAarch64),
    ("x86_64-unknown-linux-musl", "x86-64", "hello-linux-x86_64", SystemTarget::LinuxX86_64),
];

/// C that the programs call: clang's own `stdint.h`, a macro the compiler defines, and a 128-bit multiply, which needs C23's
/// `_BitInt` on a 32-bit processor, as CORE-MATH does.
const C_SOURCE: &str = "#include <stdint.h>\n\
#ifndef ANSWER\n#error \"ANSWER isn't defined\"\n#endif\n\
typedef unsigned _BitInt(128) u128;\n\
int32_t answer(void) { return ANSWER; }\n\
uint32_t next_count(uint32_t value) { u128 wide = value; return (uint32_t)((wide * wide) >> 64) + value + 1; }\n";

/// Compiles the C file for `target`, defining `ANSWER` as 49.
fn c_object(folder: &Path, target: SystemTarget, name: &str) -> PathBuf {
    let source = folder.join("answer.c");
    write(&source, C_SOURCE.as_bytes());
    let object = folder.join(format!("{name}-answer.o"));
    let options = CCompileOptions { defines: &[("ANSWER", Some("49"))], ..CCompileOptions::default() };
    compile_c(target, &source, &object, &options).unwrap_or_else(|error| panic!("{name}: {}", error.message()));
    object
}

/// `main` calls `printf` with a greeting that says where it runs, and the number the C file's `answer` gives.
fn hello_object(triple: &str, cpu: &str, said: &str) -> Vec<u8> {
    with_context(|context| {
        let machine = Target::from_triple(triple).expect("the target").create_target_machine(
            triple,
            cpu,
            "",
            CodeGenerationOptimizationLevel::Default,
            RelocationMode::Static,
            CodeModel::Default,
        );
        let mut owned = context.create_module("hello");
        let module = context.module(&owned);
        module.set_target_machine(&machine);
        let i32_type = context.get_type::<I32>();
        let pointer = context.pointer_type(0).to_dynamic();
        let printf = module.add_function("printf", context.function_type(i32_type.to_dynamic(), &[pointer], true));
        let answer = module.add_function("answer", context.function_type(i32_type.to_dynamic(), &[], false));
        let main = module.add_function("main", context.function_type(i32_type.to_dynamic(), &[], false));
        let builder = Builder::at_end_of(main.append_basic_block("entry"));
        let text =
            builder.global_string_pointer(&format!("hello from a compiler built on Lyftar, {said}, %d\n"), "greeting");
        let number = builder.call(answer, &[] as &[lyftar::Value<'_, '_>], "answer").to_dynamic();
        builder.call(printf, &[text.to_dynamic(), number], "");
        builder.return_value(i32_type.constant_integer(0, false));
        drop(builder);
        module.verify().expect("valid IR");
        machine
            .emit_to_memory_buffer(context.exclusive_module(&mut owned), CodeGenerationFileType::Object)
            .expect("code generation")
            .as_bytes()
            .to_vec()
    })
}

/// A vector table at the start of flash, with the stack's top and the reset handler, which counts forever, through the C file's
/// `next_count`. The front end's own startup code, as D49 leaves it.
fn cortex_m4_object() -> Vec<u8> {
    with_context(|context| {
        let triple = "thumbv7em-none-eabihf";
        let machine = Target::from_triple(triple).expect("the ARM target").create_target_machine(
            triple,
            "cortex-m4",
            "",
            CodeGenerationOptimizationLevel::Default,
            RelocationMode::Static,
            CodeModel::Default,
        );
        let owned = context.parse_ir_text(
            "@vectors = constant [2 x ptr] [ptr inttoptr (i32 536936448 to ptr), ptr @reset], section \".isr_vector\", align 4\n\
             @counter = global i32 0\n\
             declare i32 @next_count(i32)\n\
             define void @reset() noreturn {\n\
             entry:\n  br label %loop\n\
             loop:\n  %value = load volatile i32, ptr @counter\n  %next = call i32 @next_count(i32 %value)\n  store volatile i32 %next, ptr @counter\n  br label %loop\n\
             }\n",
        );
        let mut owned = owned.expect("good text");
        context.module(&owned).set_target_machine(&machine);
        machine
            .emit_to_memory_buffer(context.exclusive_module(&mut owned), CodeGenerationFileType::Object)
            .expect("code generation")
            .as_bytes()
            .to_vec()
    })
}

const LINKER_SCRIPT: &str = "MEMORY { FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 512K  RAM (rwx) : ORIGIN = 0x20000000, LENGTH = 128K }\n\
ENTRY(reset)\n\
SECTIONS {\n\
  .isr_vector : { KEEP(*(.isr_vector)) } > FLASH\n\
  .text : { *(.text*) } > FLASH\n\
  .data : { *(.data*) } > RAM AT > FLASH\n\
  .bss : { *(.bss*) *(COMMON) } > RAM\n\
}\n";

fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

fn main() {
    let folder = PathBuf::from(std::env::args().nth(1).expect("a folder to write into"));
    std::fs::create_dir_all(&folder).expect("the folder");
    for (triple, cpu, name, target) in HELLOS {
        let object = folder.join(format!("{name}.o"));
        write(&object, &hello_object(triple, cpu, name));
        let c_object = c_object(&folder, target, name);
        link(target, &[&object, &c_object], &folder.join(name), None)
            .unwrap_or_else(|error| panic!("{name}: {}", error.message()));
        println!("linked {name}");
    }
    let object = folder.join("cortex-m4.o");
    write(&object, &cortex_m4_object());
    let script = folder.join("cortex-m4.ld");
    write(&script, LINKER_SCRIPT.as_bytes());
    let c_object = c_object(&folder, SystemTarget::CortexM4, "cortex-m4");
    link(SystemTarget::CortexM4, &[&object, &c_object], &folder.join("cortex-m4.elf"), Some(&script))
        .unwrap_or_else(|error| panic!("cortex-m4: {}", error.message()));
    println!("linked cortex-m4.elf");
    // Twice in one process, as a compiler that builds several programs does (D52).
    link(SystemTarget::CortexM4, &[&object, &c_object], &folder.join("cortex-m4-again.elf"), Some(&script))
        .unwrap_or_else(|error| panic!("cortex-m4 again: {}", error.message()));
    println!("linked cortex-m4-again.elf");
}
