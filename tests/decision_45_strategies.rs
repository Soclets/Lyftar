//! LLVM's built-in garbage collection strategies, through real LLVM (D45).

use lyftar::garbage_collection::{SafepointPlacement, statepoint_pipeline, strategy_is_registered};
use lyftar::memory_buffer::MemoryBuffer;
use lyftar::object::Binary;
use lyftar::passes::PassOptions;
use lyftar::target::{
    CodeGenerationFileType, CodeGenerationOptimizationLevel, CodeModel, RelocationMode, Target, TargetMachine,
};
use lyftar::{AddressSpace, Builder, Context, Function, OwnedModule, Pointer, Void, with_context};

type ManagedPointer = Pointer<AddressSpace<1>>;

fn machine() -> TargetMachine {
    TargetMachine::for_host().expect("a machine")
}

#[test]
fn decision_45_the_built_in_strategies_are_registered_and_a_typo_is_not() {
    for name in ["statepoint-example", "coreclr", "shadow-stack", "ocaml", "erlang"] {
        assert!(strategy_is_registered(name), "{name}");
    }
    assert!(!strategy_is_registered("statepoint-exmaple"));
}

#[test]
#[should_panic(expected = "set_garbage_collector: no garbage collection strategy is called \"statepoint-exmaple\"")]
fn decision_45_a_strategy_nothing_registered_panics() {
    with_context(|context| {
        let module = context.create_module("typo");
        let module = context.module(&module);
        module.add_specific_function::<Function<Void, ()>>("f").set_garbage_collector("statepoint-exmaple");
    });
}

/// What `allocate` gives is held across `safepoint` and passed to `consume`, in a function using this strategy.
fn statepoint_object(strategy: &str, optimizations: &str) -> (String, Vec<u8>) {
    with_context(|context| {
        let machine = machine();
        let mut owned = context.create_module("statepoints");
        let module = context.module(&owned);
        module.set_target_machine(&machine);
        let allocate = module.add_specific_function::<Function<ManagedPointer, ()>>("allocate");
        let safepoint = module.add_specific_function::<Function<Void, ()>>("safepoint");
        let consume = module.add_specific_function::<Function<Void, (ManagedPointer,)>>("consume");
        let program = module.add_specific_function::<Function<Void, ()>>("program");
        program.set_garbage_collector(strategy);
        let builder = Builder::at_end_of(program.append_basic_block("entry"));
        let first = builder.call(allocate, (), "first");
        let second = builder.call(allocate, (), "second");
        builder.call(safepoint, (), "");
        builder.call(consume, (first,), "");
        builder.call(consume, (second,), "");
        builder.return_void();
        drop(builder);
        context
            .exclusive_module(&mut owned)
            .run_passes(
                &statepoint_pipeline(optimizations, SafepointPlacement::CallsOnly),
                Some(&machine),
                &PassOptions::default(),
            )
            .expect("the pipeline");
        let printed = context.module(&owned).print_to_string();
        let object = machine
            .emit_to_memory_buffer(context.exclusive_module(&mut owned), CodeGenerationFileType::Object)
            .expect("code generation");
        (printed, object.as_bytes().to_vec())
    })
}

fn section_named(object: &[u8], wanted: &str) -> Option<Vec<u8>> {
    let buffer = MemoryBuffer::from_bytes(object, "object");
    let binary = Binary::new(&buffer, None).expect("an object file");
    let mut sections = binary.sections();
    while !sections.is_at_end() {
        if sections.name().contains(wanted) {
            return Some(sections.contents().to_vec());
        }
        sections.move_to_next();
    }
    None
}

fn has_symbol(object: &[u8], wanted: &str) -> bool {
    let buffer = MemoryBuffer::from_bytes(object, "object");
    let binary = Binary::new(&buffer, None).expect("an object file");
    let mut symbols = binary.symbols();
    while !symbols.is_at_end() {
        if symbols.name().contains(wanted) {
            return true;
        }
        symbols.move_to_next();
    }
    false
}

#[test]
fn decision_45_statepoint_example_rewrites_managed_pointers_and_makes_a_stack_map() {
    for optimizations in ["", "default<O2>"] {
        let (printed, object) = statepoint_object("statepoint-example", optimizations);
        assert!(printed.contains("gc.statepoint"), "{printed}");
        assert!(printed.contains("gc.relocate"), "{printed}");
        let section = section_named(&object, "llvm_stackmaps").expect("a stack map section");
        assert_eq!(section[0], 3, "stack map version 3");
    }
}

#[test]
fn decision_45_coreclr_rewrites_managed_pointers_and_makes_a_stack_map() {
    let (printed, object) = statepoint_object("coreclr", "default<O2>");
    assert!(printed.contains("gc.relocate"), "{printed}");
    assert!(section_named(&object, "llvm_stackmaps").is_some());
}

/// A function with one rooted slot, through `gcroot`, `gcread` and `gcwrite`.
fn roots_module<'context>(
    context: &Context<'context>,
    machine: &TargetMachine,
    strategy: &str,
) -> OwnedModule<'context> {
    let owned = context.create_module("Roots");
    let module = context.module(&owned);
    module.set_target_machine(machine);
    let pointer = context.get_type::<Pointer>();
    let allocate = module.add_specific_function::<Function<Pointer, ()>>("allocate");
    let safepoint = module.add_specific_function::<Function<Void, ()>>("safepoint");
    let consume = module.add_specific_function::<Function<Void, (Pointer,)>>("consume");
    let program = module.add_specific_function::<Function<Void, ()>>("program");
    program.set_garbage_collector(strategy);
    let builder = Builder::at_end_of(program.append_basic_block("entry"));
    let slot = builder.stack_allocate(pointer, "slot");
    builder.garbage_collection_root(slot, pointer.constant_null());
    builder.store(builder.call(allocate, (), "made"), slot);
    builder.call(safepoint, (), "");
    let object = builder.load(pointer, slot, "object");
    let field = builder.garbage_collection_read(object, object, "field");
    builder.garbage_collection_write(field, object, object);
    builder.call(consume, (object,), "");
    builder.return_void();
    drop(builder);
    assert_eq!(module.verify(), Ok(()));
    owned
}

fn root_object(strategy: &str) -> (String, Vec<u8>) {
    with_context(|context| {
        let machine = machine();
        let mut owned = roots_module(context, &machine, strategy);
        let printed = context.module(&owned).print_to_string();
        let object = machine
            .emit_to_memory_buffer(context.exclusive_module(&mut owned), CodeGenerationFileType::Object)
            .expect("code generation");
        (printed, object.as_bytes().to_vec())
    })
}

#[test]
fn decision_45_shadow_stack_lowers_roots_to_a_chain() {
    let (printed, object) = root_object("shadow-stack");
    assert!(printed.contains("llvm.gcroot"), "{printed}");
    assert!(printed.contains("llvm.gcread"), "{printed}");
    assert!(printed.contains("llvm.gcwrite"), "{printed}");
    assert!(has_symbol(&object, "llvm_gc_root_chain"), "the chain's head");
}

#[test]
fn decision_45_ocaml_emits_its_frame_table() {
    let (_, object) = root_object("ocaml");
    assert!(has_symbol(&object, "camlRoots__frametable"), "the frame table");
}

/// `erlang`'s table only fits ELF: its printer writes an ELF section whatever the format, which corrupts a Mach-O stream.
#[test]
fn decision_45_erlang_emits_its_frame_table_for_elf_and_is_an_error_otherwise() {
    let object = with_context(|context| {
        let machine_for = |triple: &str| {
            Target::from_triple(triple).expect("LLVM's target").create_target_machine(
                triple,
                "",
                "",
                CodeGenerationOptimizationLevel::Default,
                RelocationMode::Default,
                CodeModel::Default,
            )
        };
        let linux = machine_for("x86_64-unknown-linux-gnu");
        let mut owned = roots_module(context, &linux, "erlang");
        let object = linux
            .emit_to_memory_buffer(context.exclusive_module(&mut owned), CodeGenerationFileType::Object)
            .expect("code generation");
        let macos = machine_for("arm64-apple-macosx");
        let error = macos
            .emit_to_memory_buffer(context.exclusive_module(&mut owned), CodeGenerationFileType::Object)
            .err()
            .expect("a table only ELF has");
        assert!(error.message().contains("whose tables only fit an ELF object"), "{}", error.message());
        object.as_bytes().to_vec()
    });
    assert!(section_named(&object, ".note.gc").is_some());
}
