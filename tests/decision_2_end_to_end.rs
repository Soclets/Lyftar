//! A program built with Lyftar, optimized, emitted as an object file, linked by the system's linker and run (the JIT's run is
//! in `decision_10_jit.rs`).

// Linking this way needs a Unix `cc`.
#![cfg(unix)]

use std::process::Command;

use lyftar::enums::IntegerPredicate;
use lyftar::passes::PassOptions;
use lyftar::target::{CodeGenerationFileType, TargetMachine};
use lyftar::{Bits, Builder, Integer, with_context};

type I32 = Integer<Bits<32>>;

// `int main(void) { sum = 0; for (i = 1; i <= 10; i++) sum += i; printf("%d\n", sum); return 0; }`
fn object_file_bytes() -> Vec<u8> {
    let machine = TargetMachine::for_host().expect("this machine has a target");
    with_context(|context| {
        let mut module = context.create_module("end_to_end");
        context.module(&module).set_target_machine(&machine);
        {
            let i32_type = context.get_type::<I32>();
            let pointer = context.pointer_type(0);
            let printf_type = context.function_type(i32_type.to_dynamic(), &[pointer.to_dynamic()], true);
            let printf = context.module(&module).add_function("printf", printf_type);
            let main_type = context.function_type(i32_type.to_dynamic(), &[], false);
            let main = context.module(&module).add_function("main", main_type);
            let entry = main.append_basic_block("entry");
            let header = main.append_basic_block("header");
            let body = main.append_basic_block("body");
            let exit = main.append_basic_block("exit");
            let builder = Builder::at_end_of(entry);
            let format = builder.global_string_pointer("%d\n", "format");
            let total_slot = builder.stack_allocate(i32_type, "total");
            let index_slot = builder.stack_allocate(i32_type, "index");
            let one = i32_type.constant_integer(1, false);
            builder.store(i32_type.constant_null(), total_slot);
            builder.store(one, index_slot);
            builder.branch(header);

            builder.position_at_end(header);
            let index = builder.load(i32_type, index_slot, "index_now");
            let ten = i32_type.constant_integer(10, false);
            let more = builder.integer_compare(IntegerPredicate::SignedLessOrEqual, index, ten, "more");
            builder.conditional_branch(more, body, exit);

            builder.position_at_end(body);
            let total = builder.load(i32_type, total_slot, "total_now");
            let total = builder.integer_add(total, index, "total_next");
            builder.store(total, total_slot);
            let next = builder.integer_add(index, one, "index_next");
            builder.store(next, index_slot);
            builder.branch(header);

            builder.position_at_end(exit);
            let total = builder.load(i32_type, total_slot, "total_end");
            builder.call(printf, &[format.to_dynamic(), total.to_dynamic()], "");
            builder.return_value(i32_type.constant_null());
        }
        assert_eq!(context.module(&module).verify(), Ok(()));
        // The stack slots are gone once it is optimized, so this really changed the program.
        assert!(context.module(&module).print_to_string().contains("alloca"));
        context
            .exclusive_module(&mut module)
            .run_passes("default<O2>", Some(&machine), &PassOptions::default())
            .expect("the pipeline");
        assert_eq!(context.module(&module).verify(), Ok(()));
        assert!(
            !context.module(&module).print_to_string().contains("alloca"),
            "{}",
            context.module(&module).print_to_string()
        );
        let buffer = machine
            .emit_to_memory_buffer(context.exclusive_module(&mut module), CodeGenerationFileType::Object)
            .expect("code generation");
        buffer.as_bytes().to_vec()
    })
}

#[test]
fn decision_2_a_program_is_built_optimized_emitted_linked_and_run() {
    let directory = std::env::temp_dir().join(format!("lyftar-end-to-end-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a temporary directory");
    let object = directory.join("program.o");
    let program = directory.join("program");
    std::fs::write(&object, object_file_bytes()).expect("writing the object file");
    let linked = Command::new("cc").arg(&object).arg("-o").arg(&program).output().expect("running cc");
    assert!(linked.status.success(), "{}", String::from_utf8_lossy(&linked.stderr));
    let ran = Command::new(&program).output().expect("running the program");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "55\n");
    assert!(ran.status.success());
    let _ = std::fs::remove_dir_all(directory);
}
