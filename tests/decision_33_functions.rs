//! Specific function signatures (D33), against real LLVM.

use std::process::Command;

use lyftar::target::{CodeGenerationFileType, TargetMachine};
use lyftar::values::CallInstruction;
use lyftar::{Bits, Builder, Function, Integer, Pointer, Value, VariadicFunction, Void, with_context};

type I1 = Integer<Bits<1>>;
type I32 = Integer<Bits<32>>;
type I64 = Integer<Bits<64>>;

#[test]
fn decision_33_a_specific_function_has_the_type_it_says() {
    with_context(|context| {
        let module = context.create_module("functions");
        let add = module.add_specific_function::<Function<I32, (I32, Pointer)>>("add");
        assert_eq!(add.value_type().print_to_string(), "i32 (i32, ptr)");
        let nothing = module.add_specific_function::<Function<Void, ()>>("nothing");
        assert_eq!(nothing.value_type().print_to_string(), "void ()");
        let print = module.add_specific_function::<VariadicFunction<I32, (Pointer,)>>("print");
        assert_eq!(print.as_function().value_type().print_to_string(), "i32 (ptr, ...)");
        let dynamic = context.function_type(
            context.get_type::<I32>().to_dynamic(),
            &[context.get_type::<I32>().to_dynamic(), context.get_type::<Pointer>().to_dynamic()],
            false,
        );
        assert_eq!(add.value_type(), dynamic);
        assert_eq!(module.verify(), Ok(()));
    });
}

#[test]
fn decision_33_a_call_gives_the_result_it_says_and_verifies() {
    with_context(|context| {
        let module = context.create_module("functions");
        let i32_type = context.get_type::<I32>();
        let add = module.add_specific_function::<Function<I32, (I32, I32)>>("add");
        let nothing = module.add_specific_function::<Function<Void, ()>>("nothing");
        let caller = module.add_specific_function::<Function<I32, ()>>("caller");
        let builder = Builder::at_end_of(caller.append_basic_block("entry"));
        let one = i32_type.constant_integer(1, false);
        let sum: Value<'_, '_, I32> = builder.call(add, (one, one), "sum");
        let twice = builder.integer_add(sum, sum, "twice");
        let done: Value<'_, '_, Void> = builder.call(nothing, (), "");
        assert!(done.as_kind::<CallInstruction>().is_some());
        builder.return_value(twice);
        assert_eq!(module.verify(), Ok(()));
        let text = module.print_to_string();
        assert!(text.contains("%sum = call i32 @add(i32 1, i32 1)"), "{text}");
        assert!(text.contains("call void @nothing()"), "{text}");
    });
}

#[test]
fn decision_33_twelve_arguments_are_specific_and_more_use_the_dynamic_form() {
    with_context(|context| {
        type T = I1;
        let module = context.create_module("functions");
        let long = module.add_specific_function::<Function<Void, (T, T, T, T, T, T, T, T, T, T, T, T)>>("long");
        let caller = module.add_specific_function::<Function<Void, ()>>("caller");
        let builder = Builder::at_end_of(caller.append_basic_block("entry"));
        let v = context.get_type::<T>().constant_integer(1, false);
        builder.call(long, (v, v, v, v, v, v, v, v, v, v, v, v), "");

        let dynamic = context.get_type::<T>().to_dynamic();
        let longer = module.add_function("longer", context.function_type(context.void_type(), &[dynamic; 13], false));
        builder.call(longer, &[v; 13], "");
        builder.return_void();
        assert_eq!(module.verify(), Ok(()));
    });
}

#[test]
fn decision_33_casts_check_the_signature() {
    with_context(|context| {
        let module = context.parse_ir_text("declare i32 @known(i32)\ndeclare i32 @print(ptr, ...)").expect("good IR");
        let known = module.get_function("known").expect("the function");
        assert!(known.to_dynamic().as_kind::<Function>().is_some());
        assert!(known.to_dynamic().as_kind::<Function<I32, (I32,)>>().is_some());
        assert!(known.to_dynamic().as_kind::<Function<I64, (I32,)>>().is_none());
        assert!(known.to_dynamic().as_kind::<Function<I32, (I32, I32)>>().is_none());
        assert!(known.to_dynamic().as_kind::<VariadicFunction<I32, (I32,)>>().is_none());
        let print = module.get_function("print").expect("the function");
        assert!(print.to_dynamic().as_kind::<Function<I32, (Pointer,)>>().is_none());
        assert!(print.to_dynamic().as_kind::<VariadicFunction<I32, (Pointer,)>>().is_some());
        // Something that isn't a function at all isn't one of these.
        let constant = context.get_type::<I32>().constant_integer(1, false);
        assert!(constant.to_dynamic().as_kind::<Function<I32, (I32,)>>().is_none());
    });
}

// A program with a specific `printf`, optimized by nothing, emitted, linked and run.
#[cfg(unix)]
#[test]
fn decision_33_a_variadic_call_runs() {
    let machine = TargetMachine::for_host().expect("this machine has a target");
    let object = with_context(|context| {
        let mut module = context.create_module("variadic");
        module.set_target_machine(&machine);
        {
            let printf = module.add_specific_function::<VariadicFunction<I32, (Pointer,)>>("printf");
            let main = module.add_specific_function::<Function<I32, ()>>("main");
            let builder = Builder::at_end_of(main.append_basic_block("entry"));
            let format = builder.global_string_pointer("%d %d\n", "format");
            let i32_type = context.get_type::<I32>();
            let first = i32_type.constant_integer(4, false).to_dynamic();
            let second = i32_type.constant_integer(2, false).to_dynamic();
            let written: Value<'_, '_, I32> = builder.call(printf, ((format,), &[first, second]), "written");
            let _ = written;
            builder.return_value(i32_type.constant_null());
        }
        assert_eq!(module.verify(), Ok(()));
        let buffer =
            machine.emit_to_memory_buffer(&mut module, CodeGenerationFileType::Object).expect("code generation");
        buffer.as_bytes().to_vec()
    });
    let directory = std::env::temp_dir().join(format!("lyftar-variadic-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a temporary directory");
    let object_path = directory.join("program.o");
    let program = directory.join("program");
    std::fs::write(&object_path, object).expect("writing the object file");
    let linked = Command::new("cc").arg(&object_path).arg("-o").arg(&program).output().expect("running cc");
    assert!(linked.status.success(), "{}", String::from_utf8_lossy(&linked.stderr));
    let ran = Command::new(&program).output().expect("running the program");
    assert_eq!(String::from_utf8_lossy(&ran.stdout), "4 2\n");
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn decision_33_many_variadic_arguments_work_past_what_fits_on_the_stack() {
    with_context(|context| {
        let module = context.create_module("functions");
        let print = module.add_specific_function::<VariadicFunction<Void, (I32,)>>("print");
        let caller = module.add_specific_function::<Function<Void, ()>>("caller");
        let builder = Builder::at_end_of(caller.append_basic_block("entry"));
        let one = context.get_type::<I32>().constant_integer(1, false);
        let rest = vec![one.to_dynamic(); 40];
        let call: Value<'_, '_, Void> = builder.call(print, ((one,), &rest), "");
        assert_eq!(call.checked_cast::<CallInstruction>().argument_count(), 41);
        builder.call(print, ((one,), &[]), "");
        builder.return_void();
        assert_eq!(module.verify(), Ok(()));
    });
}
