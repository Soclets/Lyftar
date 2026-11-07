//! The stack map reader against LLVM's own, on real objects, and against bad bytes (D3, D22, D24, D45, D46).

#![cfg(unix)]

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use lyftar::enums::IntegerPredicate;
use lyftar::garbage_collection::{SafepointPlacement, StatepointCall, StatepointFlags, statepoint_pipeline};
use lyftar::passes::PassOptions;
use lyftar::target::{CodeGenerationFileType, TargetMachine};
use lyftar::{AddressSpace, Bits, Builder, Function, Integer, Pointer, Void, with_context};
use lyftar_stack_maps::{Location, Root, StackMapError, StackMaps, Table, parse_section, section_in_file};

type ManagedPointer = Pointer<AddressSpace<1>>;
type I8 = Integer<Bits<8>>;
type I64 = Integer<Bits<64>>;

/// An object with one function that walks a pointer through an object across calls, which makes real derived pointers, and one
/// with a statepoint written directly, with deoptimization values and a stack allocation in its live list.
fn object_file(module_name: &str, optimizations: &str) -> Vec<u8> {
    with_context(|context| {
        let machine = TargetMachine::for_host().expect("a machine");
        let mut owned = context.create_module(module_name);
        let module = context.module(&owned);
        module.set_target_machine(&machine);
        let safepoint = module.add_specific_function::<Function<Void, ()>>("safepoint");
        let walk = module.add_specific_function::<Function<I64, (ManagedPointer, I64)>>(&format!("{module_name}_walk"));
        walk.set_garbage_collector("statepoint-example");
        let i64_type = context.get_type::<I64>();
        let entry = walk.append_basic_block("entry");
        let body = walk.append_basic_block("body");
        let done = walk.append_basic_block("done");
        let builder = Builder::at_end_of(entry);
        let object = walk.parameter(0).checked_cast::<ManagedPointer>();
        let count = walk.parameter(1).checked_cast::<I64>();
        builder.branch(body);
        builder.position_at_end(body);
        let cursor = builder.phi(context.get_type::<ManagedPointer>(), "cursor");
        let index = builder.phi(i64_type, "index");
        let sum = builder.phi(i64_type, "sum");
        let cursor_value = cursor.as_value();
        builder.call(safepoint, (), "");
        let read = builder.load(i64_type, cursor_value, "read");
        let new_sum = builder.integer_add(sum.as_value(), read, "new_sum");
        let next = builder.get_element_pointer(
            context.get_type::<I8>().to_dynamic(),
            cursor_value,
            &[i64_type.constant_integer(8, false)],
            "next",
        );
        let new_index = builder.integer_add(index.as_value(), i64_type.constant_integer(1, false), "new_index");
        let again = builder.integer_compare(IntegerPredicate::UnsignedLessThan, new_index, count, "again");
        builder.conditional_branch(again, body, done);
        cursor.add_incoming(&[(object, entry), (next, body)]);
        index.add_incoming(&[(i64_type.constant_integer(0, false), entry), (new_index, body)]);
        sum.add_incoming(&[(i64_type.constant_integer(0, false), entry), (new_sum, body)]);
        builder.position_at_end(done);
        builder.call(safepoint, (), "");
        builder.return_value(new_sum);
        drop(builder);

        let direct =
            module.add_specific_function::<Function<Void, (ManagedPointer, I64)>>(&format!("{module_name}_direct"));
        direct.set_garbage_collector("statepoint-example");
        let builder = Builder::at_end_of(direct.append_basic_block("entry"));
        let slot = builder.stack_allocate(context.get_type::<ManagedPointer>(), "slot").to_dynamic();
        let object = direct.parameter(0);
        let state = direct.parameter(1);
        let large = i64_type.constant_integer(5_000_000_000, false).to_dynamic();
        let seven = context.get_type::<Integer<Bits<32>>>().constant_integer(7, false).to_dynamic();
        let statepoint = builder.garbage_collection_statepoint(
            &StatepointCall {
                id: 42,
                patch_size_in_bytes: 0,
                function_type: safepoint.value_type(),
                callee: safepoint.to_dynamic(),
                flags: StatepointFlags::NONE,
                arguments: &[],
                transition: &[],
                deoptimization: &[state, seven, large],
                live: &[object, slot],
            },
            "",
        );
        builder.garbage_collection_relocate(statepoint, 0, 0, "relocated");
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
        let object = machine
            .emit_to_memory_buffer(context.exclusive_module(&mut owned), CodeGenerationFileType::Object)
            .expect("code generation");
        object.as_bytes().to_vec()
    })
}

fn temporary_directory(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("lyftar-stack-maps-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a temporary directory");
    directory
}

fn llvm_tool(name: &str) -> PathBuf {
    let llvm_config =
        std::env::var_os("LYFTAR_LLVM_CONFIG").unwrap_or_else(|| "/opt/homebrew/opt/llvm/bin/llvm-config".into());
    let binaries = Command::new(llvm_config).arg("--bindir").output().expect("llvm-config");
    Path::new(String::from_utf8_lossy(&binaries.stdout).trim()).join(name)
}

/// What `llvm-readobj --stackmap` says, from its version line on.
fn llvm_says(object: &Path) -> String {
    let output = Command::new(llvm_tool("llvm-readobj")).arg("--stackmap").arg(object).output().expect("llvm-readobj");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let start = text.find("LLVM StackMap Version").expect("a stack map");
    text[start..].to_string()
}

/// The same text, from what the reader read.
fn reader_says(table: &Table<'_>) -> Result<String, StackMapError> {
    let mut text = String::from("LLVM StackMap Version: 3\n");
    let _ = writeln!(text, "Num Functions: {}", table.function_count());
    for index in 0..table.function_count() {
        let function = table.function(index)?;
        let _ = writeln!(
            text,
            "  Function address: {}, stack size: {}, callsite record count: {}",
            function.address, function.stack_size_in_bytes, function.record_count
        );
    }
    let _ = writeln!(text, "Num Constants: {}", table.constant_count());
    for index in 0..table.constant_count() {
        let _ = writeln!(text, "  #{}: {}", index + 1, table.constant(index)?);
    }
    let _ = writeln!(text, "Num Records: {}", table.record_count());
    for record in table.records() {
        let _ = writeln!(
            text,
            "  Record ID: {}, instruction offset: {}",
            record.id(),
            record.instruction_offset_in_bytes()
        );
        let _ = writeln!(text, "    {} locations:", record.location_count());
        for (index, location) in record.locations().enumerate() {
            let (described, size) = match location {
                Location::Register { register, size_in_bytes } => (format!("Register R#{register}"), size_in_bytes),
                Location::Direct { register, offset_in_bytes, size_in_bytes } => {
                    (format!("Direct R#{register} + {offset_in_bytes}"), size_in_bytes)
                }
                Location::Indirect { register, offset_in_bytes, size_in_bytes } => {
                    (format!("Indirect [R#{register} + {offset_in_bytes}]"), size_in_bytes)
                }
                Location::Constant { value, size_in_bytes } => {
                    (format!("Constant {}", value.cast_unsigned()), size_in_bytes)
                }
                Location::LargeConstant { index, value, size_in_bytes } => {
                    (format!("ConstantIndex #{index} ({value})"), size_in_bytes)
                }
            };
            let _ = writeln!(text, "      #{}: {described}, size: {size}", index + 1);
        }
        let _ = write!(text, "    {} live-outs: [ ", record.live_outs().len());
        for (register, size) in record.live_outs() {
            let _ = write!(text, "R#{register} ({size}-bytes) ");
        }
        text.push_str("]\n");
    }
    Ok(text)
}

#[test]
fn decision_45_the_reader_agrees_with_llvm_readobj() {
    for (name, optimizations) in [("plain", ""), ("optimized", "default<O2>")] {
        let directory = temporary_directory(name);
        let path = directory.join("program.o");
        let object = object_file(name, optimizations);
        std::fs::write(&path, &object).expect("writing it");
        let section = section_in_file(&object).expect("good headers").expect("a stack map");
        let tables = parse_section(section).expect("a good stack map");
        assert_eq!(tables.len(), 1);
        assert_eq!(reader_says(&tables[0]).expect("a good stack map"), llvm_says(&path), "{name}");
        let _ = std::fs::remove_dir_all(directory);
    }
}

#[test]
fn decision_45_derived_pointers_differ_from_their_bases_and_allocations_stand_alone() {
    let object = object_file("derived", "");
    let section = section_in_file(&object).expect("good headers").expect("a stack map");
    let tables = parse_section(section).expect("a good stack map");
    let (mut pairs_that_differ, mut allocations, mut deoptimization_values) = (0, 0, 0);
    for record in tables[0].records() {
        let statepoint = record.statepoint().expect("every record here is a statepoint's");
        deoptimization_values += statepoint.deoptimization().len();
        for root in statepoint.roots() {
            match root {
                Root::Pair { base, derived } => pairs_that_differ += u32::from(base != derived),
                Root::Allocation(_) => allocations += 1,
            }
        }
    }
    assert!(pairs_that_differ > 0, "the walk holds a derived pointer in a slot of its own");
    assert_eq!(allocations, 1);
    assert_eq!(deoptimization_values, 3);
}

#[test]
fn decision_45_several_translation_units_read_as_one_table_each() {
    let directory = temporary_directory("several");
    let mut text = String::new();
    let mut paths = Vec::new();
    for name in ["first", "second", "third"] {
        let path = directory.join(format!("{name}.o"));
        std::fs::write(&path, object_file(name, "")).expect("writing it");
        text.push_str(&llvm_says(&path));
        paths.push(path);
    }
    let merged = directory.join("merged.o");
    let linked = Command::new("ld").arg("-r").args(&paths).arg("-o").arg(&merged).output().expect("running ld");
    assert!(linked.status.success(), "{}", String::from_utf8_lossy(&linked.stderr));
    let file = std::fs::read(&merged).expect("the merged object");
    let section = section_in_file(&file).expect("good headers").expect("a stack map");
    let tables = parse_section(section).expect("a good stack map");
    assert_eq!(tables.len(), 3);
    let read: String = tables.iter().map(|table| reader_says(table).expect("a good stack map")).collect();
    assert_eq!(read, text);
    let _ = std::fs::remove_dir_all(directory);
}

/// A small stack map with made-up addresses: one function at 0x1000, with a statepoint's record returning to 0x1010.
fn made_up_table(address: u64, id: u64) -> Vec<u8> {
    let mut bytes = vec![3, 0, 0, 0];
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(address.to_le_bytes());
    bytes.extend(32u64.to_le_bytes());
    bytes.extend(1u64.to_le_bytes());
    bytes.extend(99u64.to_le_bytes());
    bytes.extend(id.to_le_bytes());
    bytes.extend(0x10u32.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(6u16.to_le_bytes());
    let mut location = |kind: u8, register: u16, value: i32| {
        bytes.push(kind);
        bytes.push(0);
        bytes.extend(8u16.to_le_bytes());
        bytes.extend(register.to_le_bytes());
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(value.to_le_bytes());
    };
    location(4, 0, 0);
    location(4, 0, 0);
    location(4, 0, 1);
    location(5, 0, 0);
    location(3, 31, 8);
    location(3, 31, 16);
    while bytes.len() % 8 != 0 {
        bytes.push(0);
    }
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    while bytes.len() % 8 != 0 {
        bytes.push(0);
    }
    bytes
}

#[test]
fn decision_45_a_return_address_finds_its_roots_across_tables() {
    let mut section = made_up_table(0x1000, 1);
    section.extend(made_up_table(0x2000, 2));
    let mut stack_maps = StackMaps::new();
    stack_maps.add_section(&section).expect("a good stack map");
    assert_eq!(stack_maps.call_count(), 2);
    assert_eq!(stack_maps.tables().len(), 2);
    assert!(stack_maps.find(0x1000).is_none());
    let call = stack_maps.find(0x2010).expect("the second table's call");
    assert_eq!(call.record.id(), 2);
    assert_eq!(call.function.address, 0x2000);
    assert_eq!(call.function.stack_size_in_bytes, 32);
    let statepoint = call.record.statepoint().expect("a statepoint");
    assert_eq!(statepoint.deoptimization_count(), 1);
    let roots: Vec<Root> = statepoint.roots().collect();
    assert_eq!(
        roots,
        [Root::Pair {
            base: Location::Indirect { register: 31, offset_in_bytes: 8, size_in_bytes: 8 },
            derived: Location::Indirect { register: 31, offset_in_bytes: 16, size_in_bytes: 8 },
        }]
    );
}

#[test]
fn decision_45_two_calls_at_one_address_is_an_error() {
    let mut section = made_up_table(0x1000, 1);
    section.extend(made_up_table(0x1000, 2));
    let error = StackMaps::new().add_section(&section).expect_err("the same address twice");
    assert!(error.problem.contains("same address"), "{error}");
}

#[test]
fn decision_45_a_stripped_function_is_left_out_of_the_index() {
    let mut stack_maps = StackMaps::new();
    let section = [made_up_table(0, 1), made_up_table(0x1000, 2)].concat();
    stack_maps.add_section(&section).expect("a good stack map");
    assert_eq!(stack_maps.call_count(), 1);
}

#[test]
fn decision_3_cutting_a_stack_map_short_anywhere_is_an_error() {
    let table = made_up_table(0x1000, 1);
    assert!(parse_section(&table).is_ok());
    for length in 1..table.len() {
        // What is left past the last location can be padding, or the end of a record that was never read.
        let short = &table[..length];
        assert!(Table::parse(short).is_err(), "{length} bytes");
    }
}

#[test]
fn decision_3_changing_any_byte_never_panics_and_every_answer_stays_in_bounds() {
    for object in [made_up_table(0x1000, 1), object_file("damaged", "")] {
        let section = section_in_file(&object).ok().flatten().map_or(object.clone(), <[u8]>::to_vec);
        for index in 0..section.len() {
            for value in [0u8, 1, 3, 5, 0x7f, 0x80, 0xff] {
                let mut damaged = section.clone();
                damaged[index] = value;
                let mut stack_maps = StackMaps::new();
                if stack_maps.add_section(&damaged).is_ok() {
                    for table in stack_maps.tables() {
                        for record in table.records() {
                            let _ = record.locations().count();
                            let _ = record.live_outs().count();
                            if let Ok(statepoint) = record.statepoint() {
                                let _ = statepoint.roots().count();
                                let _ = statepoint.deoptimization().count();
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn decision_3_bad_files_are_errors() {
    assert!(section_in_file(b"not an object").is_err());
    assert!(section_in_file(&[]).is_err());
    assert!(section_in_file(&[0x7f, b'E', b'L', b'F']).is_err());
    assert!(section_in_file(&[0xcf, 0xfa, 0xed, 0xfe]).is_err());
    let object = object_file("cut", "");
    for length in (0..object.len()).step_by(64) {
        let _ = section_in_file(&object[..length]);
    }
    for (bytes, problem) in [
        (&[][..], "16-byte header"),
        (&[2; 16][..], "version 3"),
        (&[3, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 0][..], "lists run past"),
        (&[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0][..], "fewer records"),
    ] {
        let error = Table::parse(bytes).expect_err("bad bytes");
        assert!(error.problem.contains(problem), "{error}");
    }
    let mut more = made_up_table(0x1000, 1);
    more[32] = 2;
    assert!(Table::parse(&more).expect_err("bad").problem.contains("more records"));
    let mut bad_kind = made_up_table(0x1000, 1);
    bad_kind[16 + 24 + 8 + 16] = 9;
    assert!(Table::parse(&bad_kind).expect_err("bad").problem.contains("kind"));
    let mut bad_index = made_up_table(0x1000, 1);
    bad_index[16 + 24 + 8 + 16 + 36 + 8] = 200;
    assert!(Table::parse(&bad_index).expect_err("bad").problem.contains("index"));
    let not_statepoint = {
        let mut bytes = made_up_table(0x1000, 1);
        bytes[16 + 24 + 8 + 16] = 3;
        bytes
    };
    let table = Table::parse(&not_statepoint).expect("a good record of another shape");
    assert!(table.records().next().expect("a record").statepoint().is_err());
}
