//! The modules that follow LLVM's C API (D16), each against real LLVM.

use lyftar::attributes::{AttributeKind, AttributeLocation};
use lyftar::blake3::Blake3Hasher;
use lyftar::debug_info::{DebugInfoBuilder, DebugInfoFlags, DwarfEmissionKind, DwarfSourceLanguage, TemporaryMetadata};
use lyftar::disassembler::Disassembler;
use lyftar::enums::{Linkage, ModuleFlagBehavior, Visibility};
use lyftar::error::Error;
use lyftar::intrinsics::Intrinsic;
use lyftar::lto::{LtoCodeGenerator, LtoModule};
use lyftar::memory_buffer::MemoryBuffer;
use lyftar::object::{Binary, BinaryType};
use lyftar::passes::PassOptions;
use lyftar::remarks::{RemarkParser, RemarkType};
use lyftar::target::{CodeGenerationFileType, Target, TargetData, TargetMachine};
use lyftar::{Bits, Builder, Integer, Module, with_context};

type I32 = Integer<Bits<32>>;
type I64 = Integer<Bits<64>>;

fn add_answer_function(module: &Module<'_>) {
    let context = module.context();
    let i32_type = context.get_type::<I32>();
    let function_type = context.function_type(i32_type.to_dynamic(), &[], false);
    let function = module.add_function("answer", function_type);
    let builder = Builder::at_end_of(function.append_block("entry"));
    builder.return_value(i32_type.constant_integer(42, false));
}

#[test]
fn decision_16_bitcode_round_trip_keeps_the_function() {
    with_context(|context| {
        let module = context.create_module("first");
        add_answer_function(&module);
        let buffer = module.write_bitcode_to_memory_buffer();
        assert!(!buffer.as_bytes().is_empty());
        let read = context.parse_bitcode(&buffer).expect("good bitcode");
        assert!(read.get_function("answer").is_some());
        assert_eq!(read.verify(), Ok(()));
        let lazy =
            context.lazy_bitcode_module(MemoryBuffer::from_bytes(buffer.as_bytes(), "lazy")).expect("good bitcode");
        assert!(lazy.get_function("answer").is_some());
    });
}

#[test]
fn decision_3_bad_bitcode_and_bad_ir_return_errors() {
    with_context(|context| {
        let bad = MemoryBuffer::from_bytes(b"not bitcode", "bad");
        assert!(context.parse_bitcode(&bad).is_err());
        assert!(context.lazy_bitcode_module(MemoryBuffer::from_bytes(b"not bitcode", "bad")).is_err());
        let error: Error = context.parse_ir_text("this is not IR").err().expect("an error");
        assert!(!error.message().is_empty());
    });
}

#[test]
fn decision_11_linking_moves_definitions_and_reports_conflicts() {
    with_context(|context| {
        let mut destination = context.create_module("destination");
        let source = context.create_module("source");
        add_answer_function(&source);
        destination.link_in(source).expect("the link");
        assert!(destination.get_function("answer").is_some());
        let conflicting = context.create_module("conflicting");
        add_answer_function(&conflicting);
        let error = destination.link_in(conflicting).expect_err("a duplicate definition");
        assert!(error.message().contains("answer"), "{}", error.message());
    });
}

#[test]
fn decision_16_target_data_and_machines_answer() {
    lyftar::target::initialize_native_target().expect("this machine's target");
    let machine = TargetMachine::for_host().expect("a machine");
    assert!(!machine.triple().is_empty());
    assert!(machine.target().has_target_machine());
    assert!(Target::all().iter().any(|target| *target == machine.target()));
    with_context(|context| {
        let i64_type = context.get_type::<I64>();
        let layout = machine.data_layout();
        assert_eq!(layout.size_of_type_in_bits(i64_type), 64);
        assert_eq!(layout.store_size_in_bytes(i64_type), 8);
        assert_eq!(layout.abi_alignment_in_bytes(i64_type), 8);
        let pair = context.struct_type(&[context.get_type::<I32>().to_dynamic(), i64_type.to_dynamic()], false);
        assert_eq!(layout.offset_of_element_in_bytes(pair, 1), 8);
        assert_eq!(layout.element_at_offset(pair, 8), 1);
        assert_eq!(layout.integer_pointer_type(context).width_in_bits(), 64);
    });
    assert!(TargetData::new("not a layout!").is_err());
    assert!(TargetData::new("e-m:o-i64:64").is_ok());
}

#[test]
fn decision_16_object_files_are_read_back_and_disassembled() {
    let machine = TargetMachine::for_host().expect("a machine");
    let triple = machine.triple();
    let object = with_context(|context| {
        let mut module = context.create_module("object");
        module.set_target_machine(&machine);
        add_answer_function(&module);
        machine.emit_to_memory_buffer(&mut module, CodeGenerationFileType::Object).expect("code generation")
    });
    let binary = Binary::new(&object, None).expect("an object file");
    assert!(matches!(binary.binary_type(), BinaryType::MachO64LittleEndian | BinaryType::Elf64LittleEndian));
    let mut text_size = 0;
    let mut text_bytes = None;
    let mut sections = binary.sections();
    while !sections.is_at_end() {
        if sections.name().contains("text") {
            text_size = sections.size_in_bytes();
            text_bytes = Some(sections.contents());
            let _ = sections.relocations();
        }
        sections.move_to_next();
    }
    assert!(text_size > 0);
    let mut names = Vec::new();
    let mut symbols = binary.symbols();
    while !symbols.is_at_end() {
        names.push(symbols.name());
        symbols.move_to_next();
    }
    assert!(names.iter().any(|name| name.contains("answer")), "{names:?}");
    let mut disassembler = Disassembler::new(&triple, "", "").expect("a disassembler");
    let (size, text) = disassembler.disassemble_instruction(text_bytes.expect("the code"), 0).expect("an instruction");
    assert!(size > 0 && !text.is_empty());
}

#[test]
fn decision_16_debug_info_builds_and_verifies() {
    with_context(|context| {
        let module = context.create_module("debug");
        let version =
            context.get_type::<I32>().constant_integer(u64::from(lyftar::support::debug_metadata_version()), false);
        module.add_module_flag(ModuleFlagBehavior::Warning, "Debug Info Version", version.as_metadata());
        let builder = DebugInfoBuilder::new(&module, true);
        let file = builder.create_file("source.lang", "/tmp");
        let unit = builder.create_compile_unit(
            DwarfSourceLanguage::C,
            file,
            "lyftar test",
            false,
            "",
            0,
            "",
            DwarfEmissionKind::Full,
            0,
            false,
            false,
            "",
            "",
        );
        let i32_debug = builder.create_basic_type("int", 32, 5, DebugInfoFlags::ZERO);
        assert_eq!(i32_debug.type_size_in_bits(), 32);
        assert_eq!(i32_debug.type_name(), "int");
        let function_debug = builder.create_subroutine_type(file, Some(i32_debug), &[], DebugInfoFlags::ZERO);
        let subprogram = builder.create_function(
            unit,
            "answer",
            "answer",
            file,
            1,
            function_debug,
            false,
            true,
            1,
            DebugInfoFlags::ZERO,
            false,
        );
        let i32_type = context.get_type::<I32>();
        let function_type = context.function_type(i32_type.to_dynamic(), &[], false);
        let function = module.add_function("answer", function_type);
        function.set_subprogram(subprogram);
        let block = function.append_block("entry");
        let code = Builder::at_end_of(block);
        let location = builder.create_debug_location(1, 2, subprogram, None);
        code.set_current_debug_location(Some(location));
        let slot = code.stack_allocate(i32_type, "slot");
        let variable =
            builder.create_auto_variable(subprogram, "slot", file, 1, i32_debug, false, DebugInfoFlags::ZERO, 0);
        let expression = builder.create_expression(&[]);
        let record = builder.insert_declare_at_end(slot, variable, expression, location, block);
        assert!(record.print_to_string().contains("no terminator"));
        let returned = code.return_value(i32_type.constant_integer(1, false));
        assert!(record.print_to_string().contains("slot"));
        assert_eq!(returned.debug_location_line(), 1);
        assert_eq!(returned.debug_location_column(), 2);
        assert_eq!(returned.debug_location_filename(), "source.lang");
        builder.finalize();
        assert_eq!(module.verify(), Ok(()));
        assert_eq!(location.location_line(), 1);
        assert_eq!(file.file_name(), "source.lang");
        assert_eq!(file.file_directory(), "/tmp");
    });
}

#[test]
fn decision_16_temporary_metadata_replaces_or_clears_its_uses() {
    with_context(|context| {
        let temporary = TemporaryMetadata::new(context, &[]);
        let node = context.metadata_node(&[temporary.as_metadata()]);
        let wrapped = node.as_value(context);
        assert!(wrapped.is_metadata_node());
        let replacement = context.metadata_string("real");
        temporary.replace_all_uses_with(replacement);
        // A temporary that goes without being replaced leaves null in what used it, not a dangling pointer.
        let dropped = TemporaryMetadata::new(context, &[]);
        let _ = context.metadata_node(&[dropped.as_metadata()]).as_value(context);
        drop(dropped);
    });
}

#[test]
fn decision_16_metadata_attributes_and_intrinsics_round_trip() {
    with_context(|context| {
        let module = context.create_module("flags");
        let flag = context.metadata_string("value");
        module.add_module_flag(ModuleFlagBehavior::Override, "my flag", flag);
        assert!(module.module_flag("my flag").is_some());
        assert_eq!(module.module_flags().len(), 1);
        let named = module.get_or_insert_named_metadata("my.list");
        named.add_operand(context.metadata_node(&[flag]).as_value(context));
        assert_eq!(named.operands().len(), 1);
        assert_eq!(named.name(), "my.list");

        let i64_type = context.get_type::<I64>();
        let function_type = context.function_type(i64_type.to_dynamic(), &[i64_type.to_dynamic()], false);
        let function = module.add_function("f", function_type);
        let kind = AttributeKind::for_name("nounwind").expect("an attribute");
        function.add_attribute(AttributeLocation::Function, context.enum_attribute(kind, 0));
        assert!(function.enum_attribute(AttributeLocation::Function, kind).is_some());
        let aligned = AttributeKind::for_name("align").expect("an attribute");
        assert_eq!(context.enum_attribute(aligned, 8).value(), Some(8));
        function.add_attribute(AttributeLocation::Function, context.string_attribute("key", "value"));
        let found = function.string_attribute(AttributeLocation::Function, "key").expect("the attribute");
        assert_eq!(found.string_value().as_deref(), Some("value"));
        function.remove_enum_attribute(AttributeLocation::Function, kind);
        assert!(function.enum_attribute(AttributeLocation::Function, kind).is_none());
        function.set_linkage(Linkage::External);
        function.set_visibility(Visibility::Hidden);
        assert_eq!(function.visibility(), Visibility::Hidden);

        let add_overflow = Intrinsic::find("llvm.sadd.with.overflow").expect("an intrinsic");
        assert!(add_overflow.is_overloaded());
        let declared = module.intrinsic_declaration(add_overflow, &[i64_type.to_dynamic()]);
        assert_eq!(declared.intrinsic(), Some(add_overflow));
        assert_eq!(
            module.intrinsic_overloaded_name(add_overflow, &[i64_type.to_dynamic()]),
            "llvm.sadd.with.overflow.i64"
        );
    });
}

const REMARKS_YAML: &str = "--- !Missed
Pass:            inline
Name:            NoDefinition
DebugLoc:        { File: file.c, Line: 3, Column: 12 }
Function:        foo
Args:
  - Callee:          bar
    DebugLoc:        { File: file.c, Line: 7, Column: 2 }
  - String:          ' will not be inlined'
...
";

#[test]
fn decision_16_remarks_are_read_back() {
    let mut parser = RemarkParser::yaml(REMARKS_YAML.as_bytes());
    let remark = parser.next_remark().expect("good YAML").expect("one remark");
    assert_eq!(remark.remark_type(), RemarkType::Missed);
    assert_eq!(remark.pass_name(), "inline");
    assert_eq!(remark.remark_name(), "NoDefinition");
    assert_eq!(remark.function_name(), "foo");
    assert_eq!(remark.debug_location().map(|location| location.line), Some(3));
    let arguments = remark.arguments();
    assert_eq!(arguments.len(), 2);
    assert_eq!(arguments[0].key, "Callee");
    assert_eq!(arguments[0].value, "bar");
    drop(remark);
    assert!(parser.next_remark().expect("the end").is_none());
    let mut bad = RemarkParser::yaml(b"!!!");
    assert!(bad.next_remark().is_err());
}

#[test]
fn decision_16_blake3_matches_the_published_vector() {
    // BLAKE3's published hash of the empty input.
    let mut hasher = Blake3Hasher::new();
    let digest = hasher.finalize();
    let printed: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(printed, "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262");
    hasher.update(b"abc");
    assert_ne!(hasher.finalize(), digest);
    hasher.reset();
    assert_eq!(hasher.finalize(), digest);
}

#[test]
fn decision_16_lto_reads_bitcode_and_makes_an_object() {
    let machine = TargetMachine::for_host().expect("a machine");
    let bitcode = with_context(|context| {
        let module = context.create_module("lto");
        module.set_target_machine(&machine);
        add_answer_function(&module);
        module.write_bitcode_to_memory_buffer().as_bytes().to_vec()
    });
    assert!(lyftar::lto::is_object_file_in_memory(&bitcode));
    let module = LtoModule::from_memory(&bitcode).expect("good bitcode");
    assert_eq!(module.target_triple(), machine.triple());
    let names: Vec<String> = (0..module.symbol_count()).map(|index| module.symbol_name(index)).collect();
    assert!(names.iter().any(|name| name.contains("answer")), "{names:?}");
    assert!(!lyftar::lto::version().is_empty());
    let mut generator = LtoCodeGenerator::new();
    generator.add_module(&bitcode, "answer").expect("adding it");
    generator.add_must_preserve_symbol("_answer");
    let object = generator.compile().expect("an object");
    assert!(!object.is_empty());
}

#[test]
fn decision_16_lto_makes_an_object_after_the_bytes_it_read_are_gone() {
    let machine = TargetMachine::for_host().expect("a machine");
    let bitcode = || {
        with_context(|context| {
            let module = context.create_module("lto");
            module.set_target_machine(&machine);
            add_answer_function(&module);
            module.write_bitcode_to_memory_buffer().as_bytes().to_vec()
        })
    };
    let mut adding = LtoCodeGenerator::new();
    adding.add_module(&bitcode(), "first").expect("good bitcode");
    assert!(adding.add_module(b"not bitcode", "second").is_err());
    adding.add_must_preserve_symbol("_answer");
    assert!(!adding.compile().expect("an object").is_empty());

    let mut setting = LtoCodeGenerator::new();
    setting.set_module(&bitcode(), "first").expect("good bitcode");
    assert!(setting.set_module(b"not bitcode", "second").is_err());
    setting.add_must_preserve_symbol("_answer");
    assert!(!setting.compile().expect("an object").is_empty());
}

// One generator's going once left a dangling handler on `libLTO`'s global context, which the next error there called.
#[test]
fn decision_16_an_error_after_a_generator_has_gone_comes_back_as_an_error() {
    drop(LtoCodeGenerator::new());
    let message = LtoModule::from_memory(b"not bitcode").err().expect("an error").message().to_string();
    assert!(!message.is_empty());
}

#[test]
fn decision_16_bytes_are_there_through_dynamic_types() {
    with_context(|context| {
        let byte = context.byte_type(8);
        assert_eq!(byte.width_in_bits(), 8);
        assert_eq!(byte.print_to_string(), "b8");
        let constant = byte.constant_byte(7);
        assert_eq!(constant.checked_cast::<lyftar::values::ConstantByte>().zero_extended_value(), 7);
        let text = byte.constant_byte_from_string("12", 10);
        assert_eq!(text.checked_cast::<lyftar::values::ConstantByte>().sign_extended_value(), 12);
    });
}

#[test]
fn decision_16_support_answers() {
    assert_eq!(lyftar::support::version().0, 23);
    assert!(lyftar::support::search_for_address_of_symbol("this_symbol_is_not_there").is_none());
}

#[test]
fn decision_11_running_llvm_passes_optimizes_real_code() {
    let machine = TargetMachine::for_host().expect("a machine");
    with_context(|context| {
        let mut module = context
            .parse_ir_text(
                "define i32 @f(i32 %x) {
               %y = add i32 %x, 1
               %z = sub i32 %y, 1
               ret i32 %z
             }",
            )
            .expect("good IR");
        module.run_passes("default<O2>", Some(&machine), &PassOptions::default()).expect("the pipeline");
        assert_eq!(module.verify(), Ok(()));
        let function = module.get_function("f").expect("the function");
        assert_eq!(function.instruction_count(), 1, "{}", module.print_to_string());
        assert!(module.run_passes("no-such-pass", None, &PassOptions::default()).is_err());
        module.run_passes_on_function("f", "instcombine", None, &PassOptions::default()).expect("the pipeline");
    });
}
