//! Every runtime check panics at the call and names the problem (D3). The last test here reads the crate's source and fails
//! if a message in it has no test, so a new check can't go without one.

use lyftar::attributes::{AttributeKind, AttributeLocation};
use lyftar::debug_info::{DebugInfoBuilder, DebugInfoFlags};
use lyftar::enums::{
    AtomicOrdering, AtomicReadModifyWriteOperation, InlineAssemblyDialect, IntegerPredicate, Linkage, Opcode,
    Visibility,
};
use lyftar::functions::CallingConvention;
use lyftar::instructions::{FastMathFlags, GetElementPointerFlags};
use lyftar::intrinsics::Intrinsic;
use lyftar::jit::{ExecutionEngine, GenericValue, Jit};
use lyftar::memory_buffer::MemoryBuffer;
use lyftar::module::ValueToErase;
use lyftar::object::Binary;
use lyftar::passes::PassOptions;
use lyftar::target::{CodeGenerationFileType, TargetData, TargetMachine};
use lyftar::values::{
    AtomicCompareExchangeInstruction, AtomicReadModifyWriteInstruction, ConditionalBranchInstruction, Constant,
    ConstantDataSequential, FenceInstruction, Function, GlobalAlias, IndirectBranchInstruction, Instruction,
    LandingPadInstruction, LoadInstruction, MetadataAsValue, ShuffleVectorInstruction, StoreInstruction,
    SwitchInstruction,
};
use lyftar::{
    BasicBlock, Bits, Builder, Context, Float, Ieee754Binary, Integer, Module, Pointer, Type, Value, with_context,
};

type I1 = Integer<Bits<1>>;
type I8 = Integer<Bits<8>>;
type I32 = Integer<Bits<32>>;
type I64 = Integer<Bits<64>>;
type F32 = Float<Ieee754Binary<Bits<32>>>;
type F64 = Float<Ieee754Binary<Bits<64>>>;

/// A test that must panic with this message, given a context.
macro_rules! panics {
    ($name:ident, $expected:literal, |$context:ident| $body:block) => {
        #[test]
        #[should_panic(expected = $expected)]
        fn $name() {
            with_context(|$context| $body);
        }
    };
}

/// `void f(i64 %0, ptr %1)` with an entry block.
fn function_in<'context, 'module>(
    module: &'module Module<'context>,
    name: &str,
) -> (Value<'context, 'module, Function>, BasicBlock<'context, 'module>, Builder<'context, 'module>) {
    let context = module.context();
    let function_type = context.function_type(
        context.void_type(),
        &[context.get_type::<I64>().to_dynamic(), context.pointer_type(0).to_dynamic()],
        false,
    );
    let function = module.add_function(name, function_type);
    let block = function.append_basic_block("entry");
    (function, block, Builder::at_end_of(block))
}

fn dynamic_i64<'context>(context: Context<'context>) -> Value<'context, 'static> {
    context.get_type::<I64>().constant_integer(1, false).to_dynamic()
}

fn dynamic_i32<'context>(context: Context<'context>) -> Value<'context, 'static> {
    context.get_type::<I32>().constant_integer(1, false).to_dynamic()
}

fn i64_vector<'context>(context: Context<'context>) -> Type<'context> {
    context.get_type::<I64>().vector_type(2)
}

// ---- Text ----

panics!(decision_3_a_nul_in_a_name_panics, "create_module: \"a\\0b\" has a NUL byte inside it", |context| {
    let _ = context.create_module("a\0b");
});

// ---- Types ----

panics!(decision_4_a_checked_cast_of_a_type_panics, "checked_cast: i64 is not a", |context| {
    let _ = context.get_type::<I64>().to_dynamic().checked_cast::<Pointer>();
});
panics!(decision_3_a_kind_check_names_what_was_wanted, "is_variadic: i64 is not a function type", |context| {
    let _ = context.get_type::<I64>().is_variadic();
});
panics!(decision_3_size_of_something_unsized_panics, "size_of: void has no size", |context| {
    let _ = context.void_type().size_of();
});
panics!(decision_3_alignment_of_something_unsized_panics, "alignment_of: void has no size", |context| {
    let _ = context.void_type().alignment_of();
});
panics!(
    decision_3_setting_a_struct_body_twice_panics,
    "set_struct_body: %pair = type { i64 } already has a body",
    |context| {
        let pair = context.create_named_struct("pair");
        pair.set_struct_body(&[context.get_type::<I64>().to_dynamic()], false);
        pair.set_struct_body(&[], false);
    }
);
panics!(
    decision_22_a_struct_element_index_out_of_bounds_panics,
    "struct_element_type_at: index 1 is out of bounds",
    |context| {
        let _ = context.struct_type(&[context.get_type::<I64>().to_dynamic()], false).struct_element_type_at(1);
    }
);
panics!(decision_3_element_type_of_a_scalar_panics, "element_type: i64 is not an array or a vector", |context| {
    let _ = context.get_type::<I64>().element_type();
});
panics!(decision_3_vector_size_of_a_scalar_panics, "vector_size_in_elements: i64 is not a vector", |context| {
    let _ = context.get_type::<I64>().vector_size_in_elements();
});
panics!(decision_3_an_array_of_void_panics, "array_type: void can't be an array's element", |context| {
    let _ = context.void_type().array_type(1);
});
panics!(decision_3_a_vector_of_nothing_panics, "vector_type: can't make a vector of 0 i64", |context| {
    let _ = context.get_type::<I64>().vector_type(0);
});
panics!(
    decision_3_a_scalable_vector_of_nothing_panics,
    "scalable_vector_type: can't make a vector of 0 i64",
    |context| {
        let _ = context.get_type::<I64>().scalable_vector_type(0);
    }
);
panics!(decision_3_a_zero_of_void_panics, "constant_null: void has no zero value", |context| {
    let _ = context.void_type().constant_null();
});
panics!(decision_3_undefined_void_panics, "undefined: void can't be the type of a value", |context| {
    let _ = context.void_type().undefined();
});
panics!(decision_3_poison_void_panics, "poison: void can't be the type of a value", |context| {
    let _ = context.void_type().poison();
});
panics!(decision_22_an_integer_of_no_bits_panics, "integer_type: 0 bits is not between 1 and 2^23 bits", |context| {
    let _ = context.integer_type(0);
});
panics!(
    decision_3_a_function_type_with_a_void_parameter_panics,
    "function_type: void can't be used in a function",
    |context| {
        let _ = context.function_type(context.void_type(), &[context.void_type()], false);
    }
);
panics!(decision_3_a_struct_of_void_panics, "struct_type: void can't go in a struct", |context| {
    let _ = context.struct_type(&[context.void_type()], false);
});
panics!(decision_3_a_bad_target_extension_type_panics, "target_extension_type: LLVM has rules for", |context| {
    // LLVM knows this one: it takes no parameters.
    let _ = context.target_extension_type("aarch64.svcount", &[context.get_type::<I64>().to_dynamic()], &[]);
});

panics!(decision_22_a_byte_of_no_bits_panics, "byte_type: 0 bits is not between 1 and 2^23 bits", |context| {
    let _ = context.byte_type(0);
});
panics!(
    decision_3_a_constant_byte_from_no_words_panics,
    "constant_byte_from_words: there must be at least one word",
    |context| {
        let _ = context.byte_type(8).constant_byte_from_words(&[]);
    }
);
panics!(
    decision_22_a_bad_byte_in_text_panics,
    "constant_byte_from_string: \"x\" is not a number in radix 10",
    |context| {
        let _ = context.byte_type(8).constant_byte_from_string("x", 10);
    }
);

// ---- Values ----

panics!(decision_4_a_checked_cast_of_a_value_panics, "checked_cast: i64 1 is not a", |context| {
    let _ = dynamic_i64(context).checked_cast::<Pointer>();
});
panics!(
    decision_11_uses_of_a_context_constant_panics,
    "uses: i64 1 belongs to the context, so its uses can be in any module: use uses_in",
    |context| {
        let _ = dynamic_i64(context).uses();
    }
);
panics!(
    decision_3_replacing_uses_with_another_type_panics,
    "replace_all_uses_with: i64 %0 and i32 1 have different types",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        function.parameter(0).replace_all_uses_with(dynamic_i32(context));
    }
);
panics!(
    decision_3_replacing_uses_of_a_constant_panics,
    "replace_all_uses_with: can't replace the uses of i64 1",
    |context| {
        dynamic_i64(context).replace_all_uses_with(context.get_type::<I64>().constant_integer(2, false));
    }
);
panics!(
    decision_22_an_operand_index_out_of_bounds_panics,
    "operand: index 2 is out of bounds for 2 operands",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let sum = builder.integer_add(function.parameter(0), function.parameter(0), "");
        let _ = sum.checked_cast::<Instruction>().operand(2);
    }
);
panics!(decision_3_changing_a_shared_constants_operand_panics, "set_operand: the operands of", |context| {
    let i64_type = context.get_type::<I64>();
    let constant = context.constant_struct(&[i64_type.constant_integer(1, false)], false);
    constant.checked_cast::<Constant>().set_operand(0, i64_type.constant_integer(2, false));
});
panics!(
    decision_3_an_operand_of_another_type_panics,
    "set_operand: i32 1 doesn't have the type of operand 0",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let sum = builder.integer_add(function.parameter(0), function.parameter(0), "");
        sum.checked_cast::<Instruction>().set_operand(0, dynamic_i32(context));
    }
);
panics!(
    decision_3_an_operand_from_another_module_panics,
    "set_operand: i64 %0 belongs to another module than",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (function, _, builder) = function_in(&module, "f");
        let (elsewhere, _, _) = function_in(&other, "g");
        let sum = builder.integer_add(function.parameter(0), function.parameter(0), "");
        sum.checked_cast::<Instruction>().set_operand(0, elsewhere.parameter(0));
    }
);
panics!(
    decision_3_an_unknown_enum_value_cant_go_back_to_llvm,
    "Linkage: LLVM's value 99 is one this Lyftar doesn't name",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        function.set_linkage(Linkage::Unknown(99));
    }
);

// ---- Build calls: positions and modules ----

panics!(
    decision_3_an_instruction_after_a_terminator_panics,
    "integer_add: the block already ends in a terminator, or the builder is before a phi",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        builder.return_void();
        let _ = builder.integer_add(function.parameter(0), function.parameter(0), "");
    }
);
panics!(decision_3_an_instruction_before_a_phi_panics, "the builder is before a phi", |context| {
    let module = context.create_module("m");
    let (function, block, builder) = function_in(&module, "f");
    let phi = builder.phi(context.get_type::<I64>(), "phi");
    builder.position(block, Some(phi));
    let _ = builder.integer_add(function.parameter(0), function.parameter(0), "");
});
panics!(
    decision_3_a_second_terminator_panics,
    "return_void: a terminator goes at the end of a block that doesn't have one yet",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        builder.return_void();
        builder.return_void();
    }
);
panics!(
    decision_3_a_value_from_another_module_panics,
    "integer_add: i64 %0 belongs to another module than this builder's",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (function, _, builder) = function_in(&module, "f");
        let (elsewhere, _, _) = function_in(&other, "g");
        let _ = builder.integer_add(function.parameter(0), elsewhere.parameter(0), "");
    }
);
panics!(
    decision_3_a_constant_reaching_another_modules_global_panics,
    "belongs to another module than this builder's",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (_, _, builder) = function_in(&module, "f");
        let global = other.add_global("g", context.get_type::<I64>());
        let address = global.as_pointer().constant_pointer_to_integer(context.get_type::<I64>());
        let _ = builder.integer_add(address, address, "");
    }
);
panics!(
    decision_3_a_block_from_another_module_panics,
    "position_at_end: block %entry belongs to another module than this builder's",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (_, _, builder) = function_in(&module, "f");
        let (_, elsewhere, _) = function_in(&other, "g");
        builder.position_at_end(elsewhere);
    }
);
panics!(decision_3_a_branch_to_another_function_panics, "branch: block %entry is in another function", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let (_, elsewhere, _) = function_in(&module, "g");
    builder.branch(elsewhere);
});
panics!(
    decision_3_a_branch_to_another_module_panics,
    "branch: block %entry belongs to another module than this builder's",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (_, _, builder) = function_in(&module, "f");
        let (_, elsewhere, _) = function_in(&other, "g");
        builder.branch(elsewhere);
    }
);
panics!(decision_3_positioning_at_an_instruction_outside_the_block_panics, "position: ", |context| {
    let module = context.create_module("m");
    let (function, block, builder) = function_in(&module, "f");
    let other = function.append_basic_block("other");
    let sum = builder.integer_add(function.parameter(0), function.parameter(0), "sum");
    builder.position(other, sum.as_kind::<Instruction>());
    let _ = block;
});
panics!(
    decision_3_positioning_before_debug_records_outside_the_block_panics,
    "position_before_debug_records: ",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let other = function.append_basic_block("other");
        let sum = builder.integer_add(function.parameter(0), function.parameter(0), "sum");
        builder.position_before_debug_records(other, sum.as_kind::<Instruction>());
    }
);

// ---- Build calls: operand kinds ----

panics!(
    decision_4_dynamic_operands_of_different_widths_panic,
    "integer_add: the operands have different types, i64 and i32",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let _ = builder.integer_add(dynamic_i64(context), dynamic_i32(context), "");
    }
);
panics!(
    decision_4_a_dynamic_pointer_given_to_integer_add_panics,
    "integer_add: ptr %1 is not an integer or a vector of integers",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let _ = builder.integer_add(function.parameter(1), function.parameter(1), "");
    }
);
panics!(
    decision_4_a_dynamic_integer_given_to_float_add_panics,
    "float_add: i64 %0 is not a float or a vector of floats",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let _ = builder.float_add(function.parameter(0), function.parameter(0), "");
    }
);
panics!(decision_4_a_dynamic_integer_given_as_a_pointer_panics, "load: i64 %0 is not a pointer", |context| {
    let module = context.create_module("m");
    let (function, _, builder) = function_in(&module, "f");
    let _ = builder.load(context.get_type::<I64>(), function.parameter(0), "");
});
panics!(
    decision_4_a_vector_given_as_a_single_integer_panics,
    "memory_set: <2 x i64> zeroinitializer is not a single integer",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let byte = context.get_type::<I8>().constant_integer(0, false);
        let _ = builder.memory_set(function.parameter(1), byte, i64_vector(context).constant_null(), 1);
    }
);
panics!(
    decision_4_a_dynamic_condition_that_isnt_a_boolean_panics,
    "conditional_branch: i64 %0 is not an i1",
    |context| {
        let module = context.create_module("m");
        let (function, block, builder) = function_in(&module, "f");
        let _ = builder.conditional_branch(function.parameter(0), block, block);
    }
);
panics!(decision_3_a_cast_llvm_forbids_panics, "zero_extend: can't cast i64 1 to i32", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let _ = builder.zero_extend(dynamic_i64(context), context.get_type::<I32>(), "");
});
panics!(decision_3_a_cast_opcode_that_isnt_a_cast_panics, "cast: can't cast i64 1 to i32", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let _ = builder.cast(Opcode::IntegerAdd, dynamic_i64(context), context.get_type::<I32>(), "");
});
panics!(
    decision_3_a_binary_operation_of_the_wrong_kind_panics,
    "binary_operation: FloatAdd doesn't take i64 1 and i64 1",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let _ = builder.binary_operation(Opcode::FloatAdd, dynamic_i64(context), dynamic_i64(context), "");
    }
);
panics!(
    decision_3_a_pointer_difference_of_something_unsized_panics,
    "pointer_difference: void has no size",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let pointer = function.parameter(1);
        let _ = builder.pointer_difference(context.void_type(), pointer, pointer, "");
    }
);
panics!(decision_3_an_either_cast_llvm_forbids_panics, "truncate_or_bit_cast: can't cast i32 1 to i64", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let _ = builder.truncate_or_bit_cast(dynamic_i32(context), context.get_type::<I64>().to_dynamic(), "");
});
panics!(decision_3_a_pointer_cast_llvm_forbids_panics, "pointer_cast: can't cast ptr %1 to float", |context| {
    let module = context.create_module("m");
    let (function, _, builder) = function_in(&module, "f");
    let _ = builder.pointer_cast(function.parameter(1), context.get_type::<F32>(), "");
});
panics!(decision_3_an_integer_cast_to_a_vector_panics, "integer_cast: can't cast i64 1 to <2 x i64>", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let _ = builder.integer_cast(dynamic_i64(context), i64_vector(context), false, "");
});
panics!(decision_3_a_float_cast_to_an_integer_panics, "float_cast: can't cast double 1.000000e+00 to i64", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let float = context.get_type::<F64>().constant_float(1.0).to_dynamic();
    let _ = builder.float_cast(float, context.get_type::<I64>().to_dynamic(), "");
});
panics!(decision_3_a_stack_allocation_of_something_unsized_panics, "stack_allocate: void has no size", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let _ = builder.stack_allocate(context.void_type(), "");
});
panics!(
    decision_22_an_alignment_that_isnt_a_power_of_two_panics_in_a_build_call,
    "memory_set: 3 is not zero or a power of two",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let byte = context.get_type::<I8>().constant_integer(0, false);
        let _ = builder.memory_set(function.parameter(1), byte, dynamic_i64(context), 3);
    }
);
panics!(decision_3_a_load_of_a_function_type_panics, "load: void (i64, ptr) can't be loaded or stored", |context| {
    let module = context.create_module("m");
    let (function, _, builder) = function_in(&module, "f");
    let _ = builder.load(function.value_type(), function.parameter(1), "");
});
panics!(decision_3_a_phi_of_void_panics, "phi: void isn't a type a value can have here", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let _ = builder.phi(context.void_type(), "");
});
panics!(
    decision_3_a_get_element_pointer_with_a_bad_struct_index_panics,
    "get_element_pointer: those indices don't fit { i64 }",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let pair = context.struct_type(&[context.get_type::<I64>().to_dynamic()], false);
        let _ =
            builder.get_element_pointer(pair, function.parameter(1), &[dynamic_i64(context), dynamic_i64(context)], "");
    }
);
panics!(
    decision_22_a_struct_get_element_pointer_out_of_bounds_panics,
    "struct_get_element_pointer: index 1 is out of bounds for { i64 }",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let pair = context.struct_type(&[context.get_type::<I64>().to_dynamic()], false);
        let _ = builder.struct_get_element_pointer(pair, function.parameter(1), 1, "");
    }
);
panics!(decision_3_a_phi_after_something_else_panics, "phi: phis must come first in a block", |context| {
    let module = context.create_module("m");
    let (function, _, builder) = function_in(&module, "f");
    let _ = builder.integer_add(function.parameter(0), function.parameter(0), "");
    let _ = builder.phi(context.get_type::<I64>(), "");
});
panics!(
    decision_3_a_call_through_something_that_isnt_a_function_type_panics,
    "call_indirect: i64 is not a function type",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let _ = builder.call_indirect::<_, lyftar::Dynamic>(
            context.get_type::<I64>().to_dynamic(),
            function.parameter(1),
            &[],
            "",
        );
    }
);
panics!(
    decision_3_a_call_with_the_wrong_number_of_arguments_panics,
    "call: void (i64, ptr) can't take 0 arguments",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let none: [Value<'_, '_>; 0] = [];
        let _ = builder.call(function, &none, "");
    }
);
panics!(
    decision_3_a_call_with_the_wrong_argument_type_panics,
    "call: argument 1, i64 1, doesn't have the type",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let _ = builder.call(function, &[dynamic_i64(context), dynamic_i64(context)], "");
    }
);
panics!(decision_3_a_select_with_a_bad_condition_panics, "select: i64 1 can't pick between", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let _ = builder.select(dynamic_i64(context), dynamic_i64(context), dynamic_i64(context), "");
});
panics!(decision_3_an_element_of_a_scalar_panics, "extract_element: can't take element i64 1 of i64 1", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let _ = builder.extract_element(dynamic_i64(context), dynamic_i64(context), "");
});
panics!(decision_3_inserting_into_a_scalar_panics, "insert_element: can't put i64 1 in i64 1 at i64 1", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let _ = builder.insert_element(dynamic_i64(context), dynamic_i64(context), dynamic_i64(context), "");
});
panics!(decision_3_shuffling_scalars_panics, "shuffle_vector: can't shuffle i64 1 and i64 1 by i64 1", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let _ = builder.shuffle_vector(dynamic_i64(context), dynamic_i64(context), dynamic_i64(context), "");
});
panics!(
    decision_22_an_aggregate_index_out_of_bounds_panics,
    "extract_value: { i64 } zeroinitializer has no element 1",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let pair = context.struct_type(&[context.get_type::<I64>().to_dynamic()], false);
        let _ = builder.extract_value(pair.constant_null(), 1, "");
    }
);
panics!(
    decision_3_inserting_the_wrong_type_into_an_aggregate_panics,
    "insert_value: i32 1 can't go at element 0 of { i64 } zeroinitializer",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let pair = context.struct_type(&[context.get_type::<I64>().to_dynamic()], false);
        let _ = builder.insert_value(pair.constant_null(), dynamic_i32(context), 0, "");
    }
);

// ---- Build calls: atomics ----

panics!(decision_3_a_weak_atomicrmw_panics, "atomic_read_modify_write: an atomicrmw can't be Unordered", |context| {
    let module = context.create_module("m");
    let (function, _, builder) = function_in(&module, "f");
    let _ = builder.atomic_read_modify_write(
        AtomicReadModifyWriteOperation::Add,
        function.parameter(1),
        dynamic_i64(context),
        AtomicOrdering::Unordered,
        false,
    );
});
panics!(
    decision_3_an_atomicrmw_of_the_wrong_kind_panics,
    "atomic_read_modify_write: FloatAdd doesn't take i64 1",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let _ = builder.atomic_read_modify_write(
            AtomicReadModifyWriteOperation::FloatAdd,
            function.parameter(1),
            dynamic_i64(context),
            AtomicOrdering::Monotonic,
            false,
        );
    }
);
panics!(
    decision_3_a_bad_cmpxchg_ordering_panics,
    "atomic_compare_exchange: a cmpxchg can't be Monotonic then Release",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let _ = builder.atomic_compare_exchange(
            function.parameter(1),
            dynamic_i64(context),
            dynamic_i64(context),
            AtomicOrdering::Monotonic,
            AtomicOrdering::Release,
            false,
        );
    }
);
panics!(
    decision_3_a_cmpxchg_of_a_float_panics,
    "atomic_compare_exchange: double 1.000000e+00 isn't an integer or a pointer",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let float = context.get_type::<F64>().constant_float(1.0);
        let _ = builder.atomic_compare_exchange(
            function.parameter(1),
            float,
            float,
            AtomicOrdering::Monotonic,
            AtomicOrdering::Monotonic,
            false,
        );
    }
);
panics!(decision_3_a_weak_fence_panics, "fence: a fence can't be Monotonic", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    builder.fence(AtomicOrdering::Monotonic, false);
});

// ---- Build calls: returns ----

panics!(
    decision_3_returning_nothing_from_a_function_with_a_value_panics,
    "return_void: this function returns a value",
    |context| {
        let module = context.create_module("m");
        let i64_type = context.get_type::<I64>().to_dynamic();
        let function = module.add_function("f", context.function_type(i64_type, &[], false));
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        builder.return_void();
    }
);
panics!(decision_3_returning_the_wrong_type_panics, "return_value: this function returns void, not i64 1", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    builder.return_value(dynamic_i64(context));
});
panics!(
    decision_3_an_aggregate_return_that_doesnt_fit_panics,
    "aggregate_return: this function returns void, which 1 values don't make",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        builder.aggregate_return(&[dynamic_i64(context)]);
    }
);

// ---- Constants ----

panics!(
    decision_3_a_constant_from_no_words_panics,
    "constant_integer_from_words: there must be at least one word",
    |context| {
        let _ = context.get_type::<I64>().constant_integer_from_words(&[]);
    }
);
panics!(
    decision_22_a_bad_number_in_text_panics,
    "constant_integer_from_string: \"12x\" is not a number in radix 10",
    |context| {
        let _ = context.get_type::<I64>().constant_integer_from_string("12x", 10);
    }
);
panics!(decision_3_a_bad_float_in_text_panics, "constant_float_from_string: \"x\" is not a number", |context| {
    let _ = context.get_type::<F64>().constant_float_from_string("x");
});
panics!(
    decision_22_too_few_words_for_a_float_panics,
    "constant_float_from_bits: double needs 1 words, not 0",
    |context| {
        let _ = context.get_type::<F64>().constant_float_from_bits(&[]);
    }
);
panics!(decision_3_a_constant_array_of_void_panics, "constant_array: void can't be an array's element", |context| {
    let none: [Value<'_, '_>; 0] = [];
    let _ = context.void_type().constant_array(&none);
});
panics!(
    decision_22_data_that_doesnt_fill_its_elements_panics,
    "constant_data_array: 3 bytes can't be elements of type i64",
    |context| {
        let _ = context.get_type::<I64>().constant_data_array(&[1, 2, 3]);
    }
);
panics!(
    decision_3_a_named_struct_with_the_wrong_count_panics,
    "constant_named_struct: { i64 } has 1 elements, not 0",
    |context| {
        let pair = context.struct_type(&[context.get_type::<I64>().to_dynamic()], false);
        let none: [Value<'_, '_>; 0] = [];
        let _ = pair.constant_named_struct(&none);
    }
);
panics!(decision_3_a_named_struct_with_the_wrong_type_panics, "constant_named_struct: i32 1 is not a i64", |context| {
    let pair = context.struct_type(&[context.get_type::<I64>().to_dynamic()], false);
    let _ = pair.constant_named_struct(&[dynamic_i32(context)]);
});
panics!(
    decision_3_inline_assembly_with_bad_constraints_panics,
    "inline_assembly: \"\" are not valid constraints for",
    |context| {
        let function_type =
            context.function_type(context.void_type(), &[context.get_type::<I64>().to_dynamic()], false);
        let _ = function_type.inline_assembly("", "", false, false, InlineAssemblyDialect::Att, false);
    }
);
panics!(
    decision_3_a_constant_struct_of_a_token_panics,
    "constant_struct: token none can't go in a struct",
    |context| {
        let _ = context.constant_struct(&[context.token_type().constant_null()], false);
    }
);
panics!(
    decision_3_a_constant_vector_of_nothing_panics,
    "constant_vector: a vector has at least one element",
    |context| {
        let none: [Value<'_, '_>; 0] = [];
        let _ = context.constant_vector(&none);
    }
);
panics!(
    decision_3_a_constant_vector_of_structs_panics,
    "constant_vector: { i64 } zeroinitializer can't be a vector's element",
    |context| {
        let pair = context.struct_type(&[context.get_type::<I64>().to_dynamic()], false);
        let _ = context.constant_vector(&[pair.constant_null()]);
    }
);
panics!(
    decision_3_a_constant_array_with_something_not_constant_panics,
    "constant_array: i64 %0 is not a constant",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        let _ = context.get_type::<I64>().constant_array(&[function.parameter(0)]);
    }
);
panics!(decision_3_a_constant_array_of_another_type_panics, "constant_array: i32 1 is not a i64", |context| {
    let _ = context.get_type::<I64>().constant_array(&[dynamic_i32(context)]);
});
panics!(
    decision_22_an_element_of_constant_data_out_of_bounds_panics,
    "element: index 3 is out of bounds for 3 elements",
    |context| {
        let text = context.constant_string(b"ab", true);
        let _ = text.checked_cast::<ConstantDataSequential>().element(3);
    }
);
panics!(decision_3_the_entry_blocks_address_panics, "address: the entry block's address can't be taken", |context| {
    let module = context.create_module("m");
    let (_, block, _) = function_in(&module, "f");
    let _ = block.address();
});
panics!(
    decision_3_a_constant_add_of_something_not_constant_panics,
    "constant_add: i64 %0 and i64 1 must both be constants",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        let parameter = function.parameter(0).checked_cast::<Integer>();
        let _ = parameter.constant_add(dynamic_i64(context).checked_cast());
    }
);
panics!(
    decision_3_a_constant_add_of_different_widths_panics,
    "constant_add: i64 1 and i32 1 have different types",
    |context| {
        let left = dynamic_i64(context).checked_cast::<Integer>();
        let right = dynamic_i32(context).checked_cast::<Integer>();
        let _ = left.constant_add(right);
    }
);
panics!(
    decision_3_a_constant_negate_of_something_not_constant_panics,
    "constant_negate: i64 %0 is not a constant",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        let _ = function.parameter(0).checked_cast::<Integer>().constant_negate();
    }
);
panics!(
    decision_22_a_constant_truncate_that_doesnt_narrow_panics,
    "constant_truncate: i64 is not narrower than 32 bits",
    |context| {
        let value = dynamic_i32(context).checked_cast::<Integer>();
        let _ = value.constant_truncate(context.get_type::<I64>());
    }
);
panics!(
    decision_3_a_constant_pointer_cast_of_something_not_constant_panics,
    "constant_pointer_to_integer: ptr %1 is not a constant",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        let _ = function.parameter(1).checked_cast::<Pointer>().constant_pointer_to_integer(context.get_type::<I64>());
    }
);
panics!(
    decision_3_a_constant_get_element_pointer_with_bad_indices_panics,
    "constant_get_element_pointer: those indices don't fit void",
    |context| {
        let null = context.pointer_type(0).constant_null();
        let _ = null.constant_get_element_pointer(
            context.void_type(),
            &[dynamic_i64(context)],
            GetElementPointerFlags::NONE,
        );
    }
);
panics!(
    decision_3_a_signed_pointer_needs_constants,
    "constant_pointer_authentication: the key and discriminator must be constants",
    |context| {
        let module = context.create_module("m");
        let i64_type = context.get_type::<I64>().to_dynamic();
        let function = module.add_function(
            "f",
            context.function_type(context.void_type(), &[context.get_type::<I32>().to_dynamic()], false),
        );
        let null = context.pointer_type(0).constant_null();
        let _ = (
            i64_type,
            null.constant_pointer_authentication(
                function.parameter(0).checked_cast(),
                context.get_type::<I64>().constant_null(),
                null,
            ),
        );
    }
);
panics!(decision_3_a_constant_bit_cast_llvm_forbids_panics, "constant_bit_cast: can't cast i64 1 to i32", |context| {
    let _ = dynamic_i64(context).constant_bit_cast(context.get_type::<I32>().to_dynamic());
});
panics!(
    decision_3_a_constant_truncate_or_bit_cast_llvm_forbids_panics,
    "constant_truncate_or_bit_cast: can't cast i32 1 to i64",
    |context| {
        let _ = dynamic_i32(context).constant_truncate_or_bit_cast(context.get_type::<I64>().to_dynamic());
    }
);
panics!(
    decision_3_a_constant_pointer_cast_of_an_integer_panics,
    "constant_pointer_cast: can't cast i64 1 to i64",
    |context| {
        let _ = dynamic_i64(context).constant_pointer_cast(context.get_type::<I64>().to_dynamic());
    }
);
panics!(
    decision_3_a_dynamic_constant_cast_of_something_not_constant_panics,
    "constant_bit_cast: i64 %0 is not a constant",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        let _ = function.parameter(0).constant_bit_cast(context.get_type::<I64>().to_dynamic());
    }
);
panics!(
    decision_3_a_constant_element_of_a_scalar_panics,
    "constant_extract_element: can't take element i64 1 of i64 1",
    |context| {
        let _ = dynamic_i64(context).constant_extract_element(dynamic_i64(context));
    }
);
panics!(
    decision_3_a_constant_insert_into_a_scalar_panics,
    "constant_insert_element: can't put i64 1 in i64 1 at i64 1",
    |context| {
        let _ = dynamic_i64(context).constant_insert_element(dynamic_i64(context), dynamic_i64(context));
    }
);
panics!(
    decision_3_a_constant_shuffle_of_scalars_panics,
    "constant_shuffle_vector: can't shuffle i64 1 and i64 1 by i64 1",
    |context| {
        let _ = dynamic_i64(context).constant_shuffle_vector(dynamic_i64(context), dynamic_i64(context));
    }
);

// ---- Attributes ----

panics!(decision_3_a_number_on_a_plain_attribute_panics, "can't hold the number 8", |context| {
    let _ = context.enum_attribute(AttributeKind::for_name("nounwind").expect("an attribute"), 8);
});
panics!(decision_3_a_type_on_a_plain_attribute_panics, "doesn't hold a type", |context| {
    let _ = context.type_attribute(AttributeKind::for_name("nounwind").expect("an attribute"), context.void_type());
});
panics!(decision_3_a_range_on_a_plain_attribute_panics, "doesn't hold a range", |context| {
    let _ = context.constant_range_attribute(AttributeKind::for_name("nounwind").expect("an attribute"), 8, &[0], &[1]);
});
panics!(
    decision_3_too_few_words_for_a_range_panics,
    "constant_range_attribute: 128 bits needs 2 words at each end",
    |context| {
        let _ =
            context.constant_range_attribute(AttributeKind::for_name("range").expect("an attribute"), 128, &[0], &[1]);
    }
);
panics!(decision_3_a_range_llvm_cant_hold_panics, "constant_range_attribute: lower and upper are equal", |context| {
    let _ = context.constant_range_attribute(AttributeKind::for_name("range").expect("an attribute"), 8, &[3], &[3]);
});
panics!(
    decision_3_an_attribute_on_a_parameter_that_isnt_there_panics,
    "add_attribute: parameter 2 is out of bounds for 2 parameters",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        let kind = AttributeKind::for_name("noundef").expect("an attribute");
        function.add_attribute(AttributeLocation::Parameter(2), context.enum_attribute(kind, 0));
    }
);
panics!(
    decision_3_an_attribute_on_an_argument_that_isnt_there_panics,
    "add_attribute: parameter 0 is out of bounds for 0 arguments",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let function_type = context.function_type(context.void_type(), &[], false);
        let callee = module.add_function("g", function_type);
        let none: [Value<'_, '_>; 0] = [];
        let call = builder.call(callee, &none, "");
        let kind = AttributeKind::for_name("noundef").expect("an attribute");
        call.add_attribute(AttributeLocation::Parameter(0), context.enum_attribute(kind, 0));
    }
);

// ---- Functions and globals ----

panics!(
    decision_3_a_calling_convention_past_llvms_panics,
    "set_calling_convention: calling convention 1024 is past LLVM's 1023",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        function.set_calling_convention(CallingConvention(1024));
    }
);
panics!(
    decision_3_hidden_on_an_internal_global_panics,
    "set_visibility: a local global can only have default visibility",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        function.set_linkage(Linkage::Internal);
        function.set_visibility(Visibility::Hidden);
    }
);
panics!(
    decision_3_getting_a_global_as_a_function_panics,
    "get_or_insert_function: g is already a global that isn't a function",
    |context| {
        let module = context.create_module("m");
        let _ = module.add_global("g", context.get_type::<I64>());
        let _ = module.get_or_insert_function("g", context.function_type(context.void_type(), &[], false));
    }
);
panics!(decision_3_a_global_of_void_panics, "add_global: void can't be a global's type", |context| {
    let module = context.create_module("m");
    let _ = module.add_global("g", context.void_type());
});
panics!(decision_11_an_alias_of_another_modules_global_panics, "add_alias: @g belongs to another module", |context| {
    let module = context.create_module("m");
    let other = context.create_module("other");
    let global = other.add_global("g", context.get_type::<I64>());
    let _ = module.add_alias(context.get_type::<I64>().to_dynamic(), 0, global, "alias");
});
panics!(decision_3_a_function_of_a_non_function_type_panics, "add_function: i64 is not a function type", |context| {
    let module = context.create_module("m");
    let _ = module.add_function("f", context.get_type::<I64>().to_dynamic());
});
panics!(
    decision_3_an_alias_of_something_not_constant_panics,
    "add_alias: ptr %1 is not a constant pointer",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        let _ = module.add_alias(context.get_type::<I64>().to_dynamic(), 0, function.parameter(1), "alias");
    }
);
panics!(decision_3_an_initializer_that_isnt_constant_panics, "set_initializer: i64 %0 is not a constant", |context| {
    let module = context.create_module("m");
    let (function, _, _) = function_in(&module, "f");
    module.add_global("g", context.get_type::<I64>()).set_initializer(function.parameter(0));
});
panics!(
    decision_3_an_initializer_of_another_type_panics,
    "set_initializer: i32 1 doesn't have the type of",
    |context| {
        let module = context.create_module("m");
        module.add_global("g", context.get_type::<I64>()).set_initializer(dynamic_i32(context));
    }
);
panics!(
    decision_11_an_initializer_from_another_module_panics,
    "set_initializer: @other_global belongs to another module",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let elsewhere = other.add_global("other_global", context.get_type::<I64>());
        module.add_global("g", context.pointer_type(0)).set_initializer(elsewhere);
    }
);
panics!(
    decision_11_an_aliasee_from_another_module_panics,
    "set_aliasee: @other_global belongs to another module",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let here = module.add_global("here", context.get_type::<I64>());
        let elsewhere = other.add_global("other_global", context.get_type::<I64>());
        let alias = module.add_alias(context.get_type::<I64>().to_dynamic(), 0, here, "alias");
        alias.checked_cast::<GlobalAlias>().set_aliasee(elsewhere);
    }
);
panics!(
    decision_11_a_resolver_from_another_module_panics,
    "set_resolver: @other_global belongs to another module",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let function_type = context.function_type(context.void_type(), &[], false);
        let resolver = module.add_function("resolver", function_type);
        let elsewhere = other.add_global("other_global", context.get_type::<I64>());
        let indirect = module.add_global_indirect_function("indirect", function_type, 0, resolver);
        indirect.set_resolver(elsewhere);
    }
);
panics!(
    decision_22_a_parameter_index_out_of_bounds_panics,
    "parameter: index 2 is out of bounds for a function of 2 parameters",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        let _ = function.parameter(2);
    }
);
panics!(
    decision_11_a_personality_from_another_module_panics,
    "set_personality: @other_global belongs to another module",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (function, _, _) = function_in(&module, "f");
        function.set_personality(other.add_global("other_global", context.get_type::<I64>()));
    }
);
panics!(decision_3_prefix_data_that_isnt_constant_panics, "set_prefix_data: i64 %0 is not a constant", |context| {
    let module = context.create_module("m");
    let (function, _, _) = function_in(&module, "f");
    function.set_prefix_data(function.parameter(0));
});
panics!(
    decision_11_prologue_data_from_another_module_panics,
    "set_prologue_data: @other_global belongs to another module",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (function, _, _) = function_in(&module, "f");
        function.set_prologue_data(other.add_global("other_global", context.get_type::<I64>()));
    }
);
panics!(
    decision_22_a_parameter_alignment_that_isnt_a_power_of_two_panics,
    "set_alignment_in_bytes: 3 is not a power of two",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        function.parameters()[1].set_alignment_in_bytes(3);
    }
);

// ---- Instructions ----

fn add_in<'context, 'module>(
    function: Value<'context, 'module, Function>,
    builder: &Builder<'context, 'module>,
) -> Value<'context, 'module, Instruction> {
    builder.integer_add(function.parameter(0), function.parameter(0), "sum").checked_cast::<Instruction>()
}

panics!(
    decision_3_a_wrap_flag_on_the_wrong_instruction_panics,
    "has_no_signed_wrap: %2 = zext i64 %0 to i128 is not an add, subtract, multiply or shift left",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let zero =
            builder.zero_extend(function.parameter(0), context.integer_type(128), "").checked_cast::<Instruction>();
        let _ = zero.has_no_signed_wrap();
    }
);
panics!(
    decision_3_fast_math_on_an_integer_panics,
    "set_fast_math_flags: %sum = add i64 %0, %0 can't have fast-math flags",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        add_in(function, &builder).set_fast_math_flags(FastMathFlags::ALL);
    }
);
panics!(
    decision_22_a_load_alignment_that_isnt_a_power_of_two_panics,
    "set_alignment_in_bytes: 6 is not a power of two",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let loaded = builder.load(context.get_type::<I64>(), function.parameter(1), "");
        loaded.checked_cast::<LoadInstruction>().set_alignment_in_bytes(6);
    }
);
panics!(decision_3_a_release_load_panics, "set_ordering: a load can't be Release", |context| {
    let module = context.create_module("m");
    let (function, _, builder) = function_in(&module, "f");
    let loaded = builder.load(context.get_type::<I64>(), function.parameter(1), "");
    loaded.checked_cast::<LoadInstruction>().set_ordering(AtomicOrdering::Release);
});
panics!(decision_3_an_acquire_store_panics, "set_ordering: a store can't be Acquire", |context| {
    let module = context.create_module("m");
    let (function, _, builder) = function_in(&module, "f");
    let stored: Value<'_, '_, StoreInstruction> = builder.store(function.parameter(0), function.parameter(1));
    stored.set_ordering(AtomicOrdering::Acquire);
});
panics!(
    decision_3_setting_a_weak_atomicrmw_ordering_panics,
    "set_ordering: an atomicrmw can't be NotAtomic",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let change: Value<'_, '_, AtomicReadModifyWriteInstruction> = builder.atomic_read_modify_write(
            AtomicReadModifyWriteOperation::Add,
            function.parameter(1),
            function.parameter(0),
            AtomicOrdering::Monotonic,
            false,
        );
        change.set_ordering(AtomicOrdering::NotAtomic);
    }
);
panics!(
    decision_3_setting_a_weak_cmpxchg_success_panics,
    "set_success_ordering: a cmpxchg can't be Unordered",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let exchange: Value<'_, '_, AtomicCompareExchangeInstruction> = builder.atomic_compare_exchange(
            function.parameter(1),
            function.parameter(0),
            function.parameter(0),
            AtomicOrdering::Monotonic,
            AtomicOrdering::Monotonic,
            false,
        );
        exchange.set_success_ordering(AtomicOrdering::Unordered);
    }
);
panics!(
    decision_3_setting_a_bad_cmpxchg_failure_panics,
    "set_failure_ordering: a failed cmpxchg can't be Release",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let exchange: Value<'_, '_, AtomicCompareExchangeInstruction> = builder.atomic_compare_exchange(
            function.parameter(1),
            function.parameter(0),
            function.parameter(0),
            AtomicOrdering::Monotonic,
            AtomicOrdering::Monotonic,
            false,
        );
        exchange.set_failure_ordering(AtomicOrdering::Release);
    }
);
panics!(decision_3_setting_a_weak_fence_ordering_panics, "set_ordering: a fence can't be Unordered", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let fence: Value<'_, '_, FenceInstruction> = builder.fence(AtomicOrdering::Acquire, false);
    fence.set_ordering(AtomicOrdering::Unordered);
});
panics!(
    decision_22_an_incoming_index_out_of_bounds_panics,
    "incoming_value: index 0 is out of bounds for 0 incoming values",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let _ = builder.phi(context.get_type::<I64>(), "").incoming_value(0);
    }
);
panics!(
    decision_3_an_incoming_value_of_another_type_panics,
    "add_incoming: i32 1 doesn't have the type of",
    |context| {
        let module = context.create_module("m");
        let (_, block, builder) = function_in(&module, "f");
        let phi = builder.phi(context.get_type::<I64>().to_dynamic(), "");
        phi.add_incoming(&[(dynamic_i32(context), block)]);
    }
);
panics!(
    decision_11_an_incoming_value_from_another_module_panics,
    "add_incoming: i64 %0 belongs to another module",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (_, block, builder) = function_in(&module, "f");
        let (elsewhere, _, _) = function_in(&other, "g");
        let phi = builder.phi(context.get_type::<I64>().to_dynamic(), "");
        phi.add_incoming(&[(elsewhere.parameter(0), block)]);
    }
);
panics!(
    decision_3_an_incoming_block_from_another_function_panics,
    "add_incoming: %entry is in another function",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let (_, elsewhere, _) = function_in(&module, "g");
        let phi = builder.phi(context.get_type::<I64>().to_dynamic(), "");
        phi.add_incoming(&[(dynamic_i64(context), elsewhere)]);
    }
);
panics!(
    decision_3_a_switch_case_of_another_type_panics,
    "add_case: i32 1 is not a constant of the switch's type",
    |context| {
        let module = context.create_module("m");
        let (function, block, builder) = function_in(&module, "f");
        let switch: Value<'_, '_, SwitchInstruction> = builder.switch(function.parameter(0), block, 1);
        switch.add_case(dynamic_i32(context), block);
    }
);
panics!(decision_3_a_switch_case_in_another_function_panics, "add_case: %entry is in another function", |context| {
    let module = context.create_module("m");
    let (function, block, builder) = function_in(&module, "f");
    let (_, elsewhere, _) = function_in(&module, "g");
    let switch = builder.switch(function.parameter(0), block, 1);
    switch.add_case(dynamic_i64(context), elsewhere);
});
panics!(decision_22_a_case_index_out_of_bounds_panics, "case_value: index 0 is out of bounds for 0 cases", |context| {
    let module = context.create_module("m");
    let (function, block, builder) = function_in(&module, "f");
    let _ = builder.switch(function.parameter(0), block, 1).case_value(0);
});
panics!(
    decision_22_setting_a_case_out_of_bounds_panics,
    "set_case_value: index 0 is out of bounds for 0 cases",
    |context| {
        let module = context.create_module("m");
        let (function, block, builder) = function_in(&module, "f");
        builder.switch(function.parameter(0), block, 1).set_case_value(0, dynamic_i64(context));
    }
);
panics!(
    decision_3_setting_a_case_of_another_type_panics,
    "set_case_value: i32 1 is not a constant of the switch's type",
    |context| {
        let module = context.create_module("m");
        let (function, block, builder) = function_in(&module, "f");
        let switch = builder.switch(function.parameter(0), block, 1);
        switch.add_case(dynamic_i64(context), block);
        switch.set_case_value(0, dynamic_i32(context));
    }
);
panics!(
    decision_3_an_indirect_destination_in_another_function_panics,
    "add_destination: %entry is in another function",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let (_, elsewhere, _) = function_in(&module, "g");
        let branch: Value<'_, '_, IndirectBranchInstruction> = builder.indirect_branch(function.parameter(1), 1);
        branch.add_destination(elsewhere);
    }
);
panics!(
    decision_11_a_condition_from_another_module_panics,
    "set_condition: %test = icmp eq i64 %0, %0 belongs to another module",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (function, block, builder) = function_in(&module, "f");
        let (elsewhere, _, other_builder) = function_in(&other, "g");
        let i1_type = context.get_type::<I1>();
        let branch: Value<'_, '_, ConditionalBranchInstruction> =
            builder.conditional_branch(i1_type.constant_integer(1, false), block, block);
        let test = other_builder.integer_compare(
            IntegerPredicate::Equal,
            elsewhere.parameter(0).checked_cast::<I64>(),
            elsewhere.parameter(0).checked_cast::<I64>(),
            "test",
        );
        branch.set_condition(test);
        let _ = function;
    }
);
panics!(
    decision_22_a_successor_index_out_of_bounds_panics,
    "successor: index 0 is out of bounds for 0 successors",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let _ = builder.return_void().successor(0);
    }
);
panics!(
    decision_22_setting_a_successor_out_of_bounds_panics,
    "set_successor: index 0 is out of bounds for 0 successors",
    |context| {
        let module = context.create_module("m");
        let (_, block, builder) = function_in(&module, "f");
        builder.return_void().set_successor(0, block);
    }
);
panics!(decision_3_a_successor_in_another_function_panics, "set_successor: %entry is in another function", |context| {
    let module = context.create_module("m");
    let (_, block, builder) = function_in(&module, "f");
    let (_, elsewhere, _) = function_in(&module, "g");
    builder.branch(block).set_successor(0, elsewhere);
});
panics!(
    decision_22_an_argument_index_out_of_bounds_panics,
    "argument: index 0 is out of bounds for 0 arguments",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let callee = module.add_function("g", context.function_type(context.void_type(), &[], false));
        let none: [Value<'_, '_>; 0] = [];
        let _ = builder.call(callee, &none, "").argument(0);
    }
);
panics!(
    decision_22_a_call_alignment_that_isnt_a_power_of_two_panics,
    "set_parameter_alignment_in_bytes: 3 is not a power of two",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let callee = module.add_function("g", context.function_type(context.void_type(), &[], false));
        let none: [Value<'_, '_>; 0] = [];
        builder.call(callee, &none, "").set_parameter_alignment_in_bytes(AttributeLocation::Return, 3);
    }
);
panics!(
    decision_3_pointing_a_call_at_another_type_panics,
    "set_called_function: g2 doesn't have the type this call is made through",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let callee = module.add_function("g", context.function_type(context.void_type(), &[], false));
        let other =
            module.add_function("g2", context.function_type(context.get_type::<I64>().to_dynamic(), &[], false));
        let none: [Value<'_, '_>; 0] = [];
        builder.call(callee, &none, "").set_called_function(other);
    }
);
panics!(
    decision_11_pointing_a_call_at_another_module_panics,
    "set_called_function: elsewhere belongs to another module",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (_, _, builder) = function_in(&module, "f");
        let function_type = context.function_type(context.void_type(), &[], false);
        let callee = module.add_function("g", function_type);
        let elsewhere = other.add_function("elsewhere", function_type);
        let none: [Value<'_, '_>; 0] = [];
        builder.call(callee, &none, "").set_called_function(elsewhere);
    }
);
panics!(
    decision_22_does_not_capture_out_of_bounds_panics,
    "does_not_capture: index 0 is out of bounds for 0 arguments",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let callee = module.add_function("g", context.function_type(context.void_type(), &[], false));
        let none: [Value<'_, '_>; 0] = [];
        let _ = builder.call(callee, &none, "").does_not_capture(0);
    }
);
panics!(
    decision_3_an_invoke_destination_in_another_function_panics,
    "set_normal_destination: %entry is in another function",
    |context| {
        let module = context.create_module("m");
        let (_, block, builder) = function_in(&module, "f");
        let (_, elsewhere, _) = function_in(&module, "g");
        let function_type = context.function_type(context.void_type(), &[], false);
        let callee = module.add_function("h", function_type);
        let none: [Value<'_, '_>; 0] = [];
        let invoke = builder.invoke(function_type, callee, &none, block, block, "");
        invoke.set_normal_destination(elsewhere);
    }
);
panics!(
    decision_22_an_indirect_destination_index_out_of_bounds_panics,
    "indirect_destination: index 0 is out of bounds for 0 destinations",
    |context| {
        let module = context.create_module("m");
        let (_, block, builder) = function_in(&module, "f");
        let function_type = context.function_type(context.void_type(), &[], false);
        let assembly = function_type.inline_assembly("", "", false, false, InlineAssemblyDialect::Att, false);
        let none: [Value<'_, '_>; 0] = [];
        let branch = builder.call_branch(
            function_type,
            assembly.to_dynamic().checked_cast::<Pointer>(),
            block,
            &[],
            &none,
            &[],
            "",
        );
        let _ = branch.indirect_destination(0);
    }
);
panics!(decision_22_a_clause_index_out_of_bounds_panics, "clause: index 0 is out of bounds for 0 clauses", |context| {
    let module = context.create_module("m");
    let (_, _, builder) = function_in(&module, "f");
    let pad: Value<'_, '_, LandingPadInstruction> = builder.landing_pad(context.struct_type(&[], false), None, 0, "");
    let _ = pad.clause(0);
});
panics!(decision_3_a_clause_that_isnt_constant_panics, "add_clause: i64 %0 is not a constant", |context| {
    let module = context.create_module("m");
    let (function, _, builder) = function_in(&module, "f");
    let pad = builder.landing_pad(context.struct_type(&[], false), None, 0, "");
    pad.add_clause(function.parameter(0));
});
panics!(
    decision_11_a_clause_from_another_module_panics,
    "add_clause: @other_global belongs to another module",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (_, _, builder) = function_in(&module, "f");
        let pad = builder.landing_pad(context.struct_type(&[], false), None, 0, "");
        pad.add_clause(other.add_global("other_global", context.get_type::<I64>()));
    }
);
panics!(
    decision_22_a_funclet_argument_index_out_of_bounds_panics,
    "argument: index 0 is out of bounds for 0 arguments",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let none: [Value<'_, '_>; 0] = [];
        let pad = builder.cleanup_pad::<_, lyftar::Dynamic>(None, &none, "");
        let _ = pad.checked_cast::<lyftar::values::FuncletPadInstruction>().argument(0);
    }
);
panics!(
    decision_22_setting_a_funclet_argument_out_of_bounds_panics,
    "set_argument: index 0 is out of bounds for 0 arguments",
    |context| {
        let module = context.create_module("m");
        let (_, _, builder) = function_in(&module, "f");
        let none: [Value<'_, '_>; 0] = [];
        let pad = builder.cleanup_pad::<_, lyftar::Dynamic>(None, &none, "");
        pad.checked_cast::<lyftar::values::FuncletPadInstruction>().set_argument(0, dynamic_i64(context));
    }
);
panics!(
    decision_11_a_funclet_argument_from_another_module_panics,
    "set_argument: i64 %0 belongs to another module",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (_, _, builder) = function_in(&module, "f");
        let (elsewhere, _, _) = function_in(&other, "g");
        let pad = builder.cleanup_pad::<_, lyftar::Dynamic>(None, &[dynamic_i64(context)], "");
        pad.checked_cast::<lyftar::values::FuncletPadInstruction>().set_argument(0, elsewhere.parameter(0));
    }
);
panics!(
    decision_22_a_mask_index_out_of_bounds_panics,
    "mask_value: index 2 is out of bounds for 2 mask elements",
    |context| {
        let module = context.create_module("m");
        let (function, _, builder) = function_in(&module, "f");
        let i32_type = context.get_type::<I32>();
        let vector_type = context.get_type::<I64>().vector_type(2);
        let phi = builder.phi(vector_type, "");
        let mask = context.constant_vector(&[i32_type.constant_integer(0, false), i32_type.constant_integer(1, false)]);
        let shuffled = builder.shuffle_vector(phi.as_value(), phi.as_value(), mask, "");
        let _ = (function, shuffled.checked_cast::<ShuffleVectorInstruction>().mask_value(2));
    }
);
panics!(
    decision_3_a_block_moved_to_another_function_panics,
    "move_before: %entry and %entry are in different functions",
    |context| {
        let module = context.create_module("m");
        let (_, block, _) = function_in(&module, "f");
        let (_, elsewhere, _) = function_in(&module, "g");
        block.move_before(elsewhere);
    }
);

// ---- Metadata ----

panics!(
    decision_3_wrapping_a_block_as_metadata_panics,
    "as_metadata: a block can't be wrapped as metadata",
    |context| {
        let module = context.create_module("m");
        let (_, block, _) = function_in(&module, "f");
        let _ = block.as_value().as_metadata();
    }
);
panics!(decision_3_operands_of_a_metadata_string_panics, "is not a metadata node", |context| {
    let _ = context.metadata_string("text").as_value(context).node_operands();
});
panics!(decision_3_replacing_an_operand_of_a_metadata_string_panics, "is not a resolved metadata node", |context| {
    let wrapped: Value<'_, '_, MetadataAsValue> = context.metadata_string("text").as_value(context);
    wrapped.replace_node_operand(0, context.metadata_string("other"));
});
panics!(
    decision_22_replacing_a_metadata_operand_out_of_bounds_panics,
    "replace_node_operand: index 1 is out of bounds for 1 operands",
    |context| {
        let node = context.metadata_node(&[context.metadata_string("text")]);
        node.as_value(context).replace_node_operand(1, context.metadata_string("other"));
    }
);
panics!(decision_3_adding_a_metadata_string_to_named_metadata_panics, "is not a metadata node", |context| {
    let module = context.create_module("m");
    let named = module.get_or_insert_named_metadata("list");
    named.add_operand(context.metadata_string("text").as_value(context));
});

// ---- Debug info ----

panics!(decision_3_debug_info_of_the_wrong_class_panics, "create_function: that metadata is not a file", |context| {
    let module = context.create_module("m");
    let builder = DebugInfoBuilder::new(&module, true);
    let file = builder.create_file("a", "b");
    let basic = builder.create_basic_type("int", 32, 5, DebugInfoFlags::ZERO);
    let _ = builder.create_function(file, "f", "f", basic, 1, basic, false, true, 1, DebugInfoFlags::ZERO, false);
});
panics!(decision_3_a_label_records_variable_panics, "variable: a label record has no variable", |context| {
    let module = context.create_module("m");
    let (function, block, code) = function_in(&module, "f");
    let builder = DebugInfoBuilder::new(&module, true);
    let file = builder.create_file("a", "b");
    let unit = builder.create_compile_unit(
        lyftar::debug_info::DwarfSourceLanguage::C,
        file,
        "",
        false,
        "",
        0,
        "",
        lyftar::debug_info::DwarfEmissionKind::Full,
        0,
        false,
        false,
        "",
        "",
    );
    let function_debug = builder.create_subroutine_type(file, None, &[], DebugInfoFlags::ZERO);
    let subprogram =
        builder.create_function(unit, "f", "f", file, 1, function_debug, false, true, 1, DebugInfoFlags::ZERO, false);
    function.set_subprogram(subprogram);
    let label = builder.create_label(subprogram, "label", file, 1, true);
    let location = builder.create_debug_location(1, 1, subprogram, None);
    let record = builder.insert_label_at_end(label, location, block);
    let _ = code;
    let _ = record.variable();
});
panics!(
    decision_22_too_few_words_for_an_enumerator_panics,
    "create_enumerator_of_arbitrary_precision: 128 bits needs 2 words, not 1",
    |context| {
        let module = context.create_module("m");
        let builder = DebugInfoBuilder::new(&module, true);
        let _ = builder.create_enumerator_of_arbitrary_precision("e", 128, &[1], false);
    }
);
panics!(
    decision_3_a_static_member_value_that_isnt_constant_panics,
    "create_static_member_type: i64 %0 isn't a constant",
    |context| {
        let module = context.create_module("m");
        let (function, _, _) = function_in(&module, "f");
        let builder = DebugInfoBuilder::new(&module, true);
        let file = builder.create_file("a", "b");
        let basic = builder.create_basic_type("int", 32, 5, DebugInfoFlags::ZERO);
        let _ = builder.create_static_member_type(
            file,
            "m",
            file,
            1,
            basic,
            DebugInfoFlags::ZERO,
            Some(function.parameter(0)),
            0,
        );
    }
);
panics!(
    decision_3_a_parameter_variable_numbered_zero_panics,
    "create_parameter_variable: argument numbers start at 1",
    |context| {
        let module = context.create_module("m");
        let builder = DebugInfoBuilder::new(&module, true);
        let file = builder.create_file("a", "b");
        let unit = builder.create_compile_unit(
            lyftar::debug_info::DwarfSourceLanguage::C,
            file,
            "",
            false,
            "",
            0,
            "",
            lyftar::debug_info::DwarfEmissionKind::Full,
            0,
            false,
            false,
            "",
            "",
        );
        let function_debug = builder.create_subroutine_type(file, None, &[], DebugInfoFlags::ZERO);
        let subprogram = builder.create_function(
            unit,
            "f",
            "f",
            file,
            1,
            function_debug,
            false,
            true,
            1,
            DebugInfoFlags::ZERO,
            false,
        );
        let basic = builder.create_basic_type("int", 32, 5, DebugInfoFlags::ZERO);
        let _ = builder.create_parameter_variable(subprogram, "p", 0, file, 1, basic, true, DebugInfoFlags::ZERO);
    }
);

// ---- Intrinsics ----

panics!(
    decision_3_an_intrinsic_on_the_wrong_overloads_panics,
    "intrinsic_declaration: llvm.sadd.with.overflow isn't overloaded on 1 types like those",
    |context| {
        let module = context.create_module("m");
        let intrinsic = Intrinsic::find("llvm.sadd.with.overflow").expect("an intrinsic");
        let _ = module.intrinsic_declaration(intrinsic, &[context.get_type::<F64>().to_dynamic()]);
    }
);

// ---- The JIT ----

panics!(
    decision_3_a_generic_value_of_an_odd_float_panics,
    "from_float: only `float` and `double`, not fp128",
    |context| {
        let _ = GenericValue::from_float(context.get_type::<Float<Ieee754Binary<Bits<128>>>>(), 1.0);
    }
);
panics!(
    decision_3_reading_a_generic_value_as_an_odd_float_panics,
    "to_float: only `float` and `double`, not fp128",
    |context| {
        let value = GenericValue::from_integer(context.get_type::<I64>(), 1, false);
        let _ = value.to_float(context.get_type::<Float<Ieee754Binary<Bits<128>>>>());
    }
);
panics!(
    decision_3_removing_a_module_the_engine_doesnt_have_panics,
    "remove_module: this engine has no module nope",
    |context| {
        let mut engine = ExecutionEngine::new(context.create_module("m")).expect("an engine");
        let _ = engine.remove_module("nope");
    }
);

#[test]
#[should_panic(expected = "register_event_listener: this JIT doesn't use RuntimeDyld")]
fn decision_3_a_listener_on_jitlink_panics() {
    let jit = Jit::new().expect("a JIT");
    jit.register_event_listener(&lyftar::jit::JitEventListener::gdb_registration());
}

// ---- Module ----

panics!(
    decision_11_a_handle_from_another_module_panics,
    "get: this handle's value belongs to another module",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let (function, _, _) = function_in(&module, "f");
        let handle = module.track(function);
        let _ = handle.get(&other);
    }
);
panics!(decision_3_a_bad_data_layout_panics, "set_data_layout: LLVM can't parse", |context| {
    context.create_module("m").set_data_layout("nonsense!");
});
panics!(decision_11_erasing_a_constant_panics, "value_to_erase: i64 1 is not an instruction", |context| {
    let module = context.create_module("m");
    let _ = module.value_to_erase(dynamic_i64(context));
});
panics!(
    decision_11_a_value_from_another_module_cant_be_chosen,
    "value_to_erase: @g belongs to another module",
    |context| {
        let module = context.create_module("m");
        let other = context.create_module("other");
        let _ = module.value_to_erase(other.add_global("g", context.get_type::<I64>()));
    }
);
panics!(
    decision_11_a_stale_entry_panics,
    "erase: this entry is stale: it was chosen before something exclusive",
    |context| {
        let mut module = context.create_module("m");
        let (entry, other): (ValueToErase, ValueToErase) = {
            let global = module.add_global("g", context.get_type::<I64>());
            let second = module.add_global("h", context.get_type::<I64>());
            (module.value_to_erase(global), module.value_to_erase(second))
        };
        // The first erase changes the module's generation, so what it did can't be known about `other`.
        module.erase(&[entry]);
        module.erase(&[other]);
    }
);
panics!(decision_11_an_entry_used_twice_panics, "erase: this entry is stale", |context| {
    let mut module = context.create_module("m");
    let entry = {
        let global = module.add_global("g", context.get_type::<I64>());
        module.value_to_erase(global)
    };
    module.erase(&[entry]);
    module.erase(&[entry]);
});
panics!(decision_11_an_entry_from_another_module_panics, "erase: this entry is stale", |context| {
    let mut module = context.create_module("m");
    let other = context.create_module("other");
    let entry = other.value_to_erase(other.add_global("g", context.get_type::<I64>()));
    module.erase(&[entry]);
});
panics!(decision_11_a_duplicate_entry_panics, "erase: the list has the same value twice", |context| {
    let mut module = context.create_module("m");
    let entry = {
        let global = module.add_global("g", context.get_type::<I64>());
        module.value_to_erase(global)
    };
    module.erase(&[entry, entry]);
});
panics!(
    decision_11_erasing_something_still_used_panics,
    "erase: @f is still used by something that stays",
    |context| {
        let mut module = context
            .parse_ir_text("define void @f() { ret void }\ndefine void @g() { call void @f()\n ret void }")
            .expect("good IR");
        let entry = {
            let function = module.get_function("f").expect("the function");
            module.value_to_erase(function)
        };
        module.erase(&[entry]);
    }
);
panics!(decision_11_erasing_a_global_an_initializer_uses_panics, "erase: @target", |context| {
    let mut module = context.parse_ir_text("@target = global i64 1\n@user = global ptr @target").expect("good IR");
    let entry = {
        let global = module.get_global("target").expect("the global");
        module.value_to_erase(global)
    };
    module.erase(&[entry]);
});
panics!(decision_11_tracking_a_constant_panics, "track: i64 1 belongs to the context and needs no handle", |context| {
    let module = context.create_module("m");
    let _ = module.track(dynamic_i64(context));
});

panics!(decision_11_a_stale_inline_choice_panics, "inline_call: this choice is stale", |context| {
    let mut module = context
        .parse_ir_text("define void @f() { ret void }\ndefine void @g() { call void @f()\n ret void }")
        .expect("good IR");
    let choice = {
        let g = module.get_function("g").expect("the function");
        let call =
            g.function_instructions().find_map(|found| found.as_kind::<lyftar::values::CallBase>()).expect("a call");
        module.call_to_inline(call)
    };
    module.strip_debug_info();
    let _ = module.inline_call(choice);
});

// ---- Object files ----

fn empty_object() -> MemoryBuffer<'static> {
    let machine = TargetMachine::for_host().expect("a machine");
    with_context(|context| {
        let mut module = context.create_module("empty");
        module.set_target_machine(&machine);
        machine.emit_to_memory_buffer(&mut module, CodeGenerationFileType::Object).expect("code generation")
    })
}

#[test]
#[should_panic(expected = "macho_universal_object_for_architecture: this is a")]
fn decision_3_a_universal_binary_call_on_another_kind_panics() {
    let object = empty_object();
    let binary = Binary::new(&object, None).expect("an object");
    let _ = binary.macho_universal_object_for_architecture("arm64");
}

#[test]
#[should_panic(expected = "sections: a Ir isn't an object file")]
fn decision_3_asking_ir_for_sections_panics() {
    with_context(|context| {
        let module = context.create_module("m");
        let bitcode = module.write_bitcode_to_memory_buffer();
        let bytes = bitcode.as_bytes().to_vec();
        let buffer = MemoryBuffer::from_bytes(&bytes, "ir");
        let leaked: &'static MemoryBuffer<'static> = Box::leak(Box::new(buffer));
        // A context can't outlive the closure, and a binary of IR wants one, so this one borrows the context's lifetime.
        fn sections_of<'context>(buffer: &'context MemoryBuffer<'context>, context: Context<'context>) {
            let binary = Binary::new(buffer, Some(context)).expect("a binary");
            let _ = binary.sections();
        }
        sections_of(leaked, context);
    });
}

#[test]
#[should_panic(expected = "name: this cursor is past the last section")]
fn decision_3_a_section_cursor_past_the_end_panics() {
    let object = empty_object();
    let binary = Binary::new(&object, None).expect("an object");
    let mut sections = binary.sections();
    while !sections.is_at_end() {
        sections.move_to_next();
    }
    let _ = sections.name();
}

#[test]
#[should_panic(expected = "name: this cursor is past the last symbol")]
fn decision_3_a_symbol_cursor_past_the_end_panics() {
    let object = empty_object();
    let binary = Binary::new(&object, None).expect("an object");
    let mut symbols = binary.symbols();
    while !symbols.is_at_end() {
        symbols.move_to_next();
    }
    let _ = symbols.name();
}

// ---- Replacing, where LLVM would delete what Rust may hold ----

#[test]
#[should_panic(expected = "replace_all_uses_with: a constant uses @first, and LLVM can delete that constant")]
fn decision_11_replacing_a_global_a_constant_uses_panics() {
    with_context(|context| {
        let module = context.parse_ir_text("@first = global i64 0\n@second = global i64 0").expect("good IR");
        let first = module.get_global("first").expect("the global");
        let second = module.get_global("second").expect("the global");
        let held = first.as_pointer().constant_pointer_to_integer(context.get_type::<I64>());
        first.replace_all_uses_with(second);
        let _ = held.print_to_string();
    });
}

#[test]
#[should_panic(expected = "replace_all_uses_with: metadata refers to %x")]
fn decision_11_replacing_a_value_wrapped_in_metadata_with_a_constant_panics() {
    with_context(|context| {
        let module =
            context.parse_ir_text("define i64 @f(i64 %a) {\n %x = add i64 %a, 1\n ret i64 %x\n}").expect("good IR");
        let function = module.get_function("f").expect("the function");
        let x = function.function_instructions().next().expect("the add");
        let held = x.as_metadata();
        x.replace_all_uses_with(context.get_type::<I64>().constant_integer(1, false));
        let _ = held.as_value(context).print_to_string();
    });
}

#[test]
#[should_panic(expected = "replace_all_uses_with: metadata refers to %x")]
fn decision_11_replacing_a_value_with_one_that_has_metadata_panics() {
    with_context(|context| {
        let module =
            context.parse_ir_text("define i64 @f(i64 %a) {\n %x = add i64 %a, 1\n ret i64 %x\n}").expect("good IR");
        let function = module.get_function("f").expect("the function");
        let x = function.function_instructions().next().expect("the add");
        let a = function.parameter(0);
        let held = x.as_metadata();
        let other = a.as_metadata();
        x.replace_all_uses_with(a);
        let _ = (held.as_value(context).print_to_string(), other.as_value(context).print_to_string());
    });
}

#[test]
#[should_panic(expected = "replace_all_uses_with: can't replace the uses of { i64 } { i64 1 } with")]
fn decision_11_replacing_a_context_constant_panics() {
    with_context(|context| {
        let i64_type = context.get_type::<I64>();
        let first = context.constant_struct(&[i64_type.constant_integer(1, false)], false);
        let second = context.constant_struct(&[i64_type.constant_integer(2, false)], false);
        first.replace_all_uses_with(second);
    });
}

// ---- Replacing with exclusive access (D34) ----

const TWO_FUNCTIONS: &str = "define i64 @f(i64 %x) {\n %y = add i64 %x, 1\n ret i64 %y\n}\n\
                             define i8 @g(i8 %small) {\n ret i8 %small\n}";

#[test]
#[should_panic(expected = "replacement_to_make: i64 %x and i8 %small have different types")]
fn decision_34_a_replacement_of_another_type_panics() {
    with_context(|context| {
        let module = context.parse_ir_text(TWO_FUNCTIONS).expect("good IR");
        let x = module.get_function("f").expect("the function").parameter(0);
        let small = module.get_function("g").expect("the function").parameter(0);
        let _ = module.replacement_to_make(x, small);
    });
}

#[test]
#[should_panic(expected = "replacement_to_make: can't replace the uses of i64 %x with i64 %x")]
fn decision_34_replacing_a_value_with_itself_panics() {
    with_context(|context| {
        let module = context.parse_ir_text(TWO_FUNCTIONS).expect("good IR");
        let x = module.get_function("f").expect("the function").parameter(0);
        let _ = module.replacement_to_make(x, x);
    });
}

#[test]
#[should_panic(expected = "replacement_to_make: can't replace the uses of { i64 } { i64 1 } with")]
fn decision_34_replacing_a_context_constant_panics() {
    with_context(|context| {
        let module = context.create_module("m");
        let i64_type = context.get_type::<I64>();
        let first = context.constant_struct(&[i64_type.constant_integer(1, false)], false);
        let second = context.constant_struct(&[i64_type.constant_integer(2, false)], false);
        let _ = module.replacement_to_make(first, second);
    });
}

#[test]
#[should_panic(expected = "replace: this entry is stale")]
fn decision_34_a_stale_replacement_panics() {
    with_context(|context| {
        let mut module = context.parse_ir_text(TWO_FUNCTIONS).expect("good IR");
        let entry = {
            let function = module.get_function("f").expect("the function");
            let y = function.function_instructions().next().expect("the add");
            module.replacement_to_make(y, function.parameter(0))
        };
        module.replace(&[entry]);
        module.replace(&[entry]);
    });
}

#[test]
#[should_panic(expected = "replace: the list replaces the same value twice")]
fn decision_34_replacing_one_value_twice_panics() {
    with_context(|context| {
        let mut module = context.parse_ir_text(TWO_FUNCTIONS).expect("good IR");
        let entry = {
            let function = module.get_function("f").expect("the function");
            let y = function.function_instructions().next().expect("the add");
            module.replacement_to_make(y, function.parameter(0))
        };
        module.replace(&[entry, entry]);
    });
}

#[test]
#[should_panic(expected = "uses a value this list replaces, and LLVM could delete it midway")]
fn decision_34_a_constant_in_the_list_that_uses_a_replaced_value_panics() {
    with_context(|context| {
        let mut module = context.parse_ir_text("@first = global i64 0\n@second = global i64 0").expect("good IR");
        let (one, two) = {
            let first = module.get_global("first").expect("the global");
            let second = module.get_global("second").expect("the global");
            let i64_type = context.get_type::<I64>();
            let of_first = first.as_pointer().constant_pointer_to_integer(i64_type);
            let of_second = second.as_pointer().constant_pointer_to_integer(i64_type);
            (module.replacement_to_make(first, second), module.replacement_to_make(of_first, of_second))
        };
        module.replace(&[one, two]);
    });
}

#[test]
#[should_panic(expected = "uses a value this list replaces, and LLVM could delete it midway")]
fn decision_34_replacing_a_value_with_a_constant_that_uses_it_panics() {
    with_context(|context| {
        let mut module = context.parse_ir_text("@first = global ptr null").expect("good IR");
        let entry = {
            let first = module.get_global("first").expect("the global");
            let pointer = first.as_pointer();
            let i64_type = context.get_type::<I64>();
            let cast = pointer.constant_pointer_to_integer(i64_type).constant_integer_to_pointer(pointer.get_type());
            module.replacement_to_make(first, cast)
        };
        module.replace(&[entry]);
    });
}

// ---- Address spaces (D31) ----

panics!(
    decision_31_a_pointer_in_too_high_a_space_panics,
    "pointer_type: address space 16777216 is not below 2^24",
    |context| {
        let _ = context.pointer_type(16_777_216);
    }
);

panics!(
    decision_31_a_global_in_too_high_a_space_panics,
    "add_global: address space 16777216 is not below 2^24",
    |context| {
        let module = context.create_module("m");
        let _ = module.add_global_in_address_space("g", context.get_type::<I64>(), 16_777_216);
    }
);

panics!(
    decision_31_an_alias_in_too_high_a_space_panics,
    "add_alias: address space 16777216 is not below 2^24",
    |context| {
        let module = context.create_module("m");
        let target = module.add_global("g", context.get_type::<I64>());
        let _ = module.add_alias(context.get_type::<I64>().to_dynamic(), 16_777_216, target, "a");
    }
);

panics!(
    decision_31_an_indirect_function_in_too_high_a_space_panics,
    "add_global_indirect_function: address space 16777216 is not below 2^24",
    |context| {
        let module = context.create_module("m");
        let function_type = context.function_type(context.void_type(), &[], false);
        let resolver = module.add_function("r", function_type);
        let _ = module.add_global_indirect_function("i", function_type, 16_777_216, resolver);
    }
);

panics!(decision_31_the_address_space_of_an_integer_panics, "address_space: i64 is not a pointer", |context| {
    let _ = context.get_type::<I64>().address_space();
});

panics!(
    decision_31_a_cast_to_an_integer_as_a_pointer_panics,
    "constant_integer_to_pointer: i64 is not a pointer type",
    |context| {
        let _ = context
            .get_type::<I64>()
            .constant_integer(1, false)
            .constant_integer_to_pointer(context.get_type::<I64>().to_dynamic());
    }
);

panics!(
    decision_31_an_address_space_cast_of_something_not_a_pointer_panics,
    "constant_address_space_cast: i64 1 is not a pointer",
    |context| {
        let value = context.get_type::<I64>().constant_integer(1, false).to_dynamic();
        let _ = value.constant_address_space_cast(context.get_type::<Pointer>());
    }
);

panics!(
    decision_31_an_address_space_cast_to_something_not_a_pointer_panics,
    "constant_address_space_cast: i64 is not a pointer type",
    |context| {
        let null = context.get_type::<Pointer>().constant_null();
        let _ = null.constant_address_space_cast(context.get_type::<I64>().to_dynamic());
    }
);

panics!(
    decision_31_an_address_space_cast_within_a_space_panics,
    "constant_address_space_cast: can't cast ptr null to ptr",
    |context| {
        let null = context.get_type::<Pointer>().constant_null();
        let _ = null.constant_address_space_cast(context.get_type::<Pointer>());
    }
);

// ---- What LLVM would crash on (D38) ----

panics!(
    decision_38_adding_an_invalid_module_to_an_engine_panics,
    "add_module: this module fails LLVM's verifier: Basic Block in function 'f' does not have terminator",
    |context| {
        let first = context.create_module("first");
        let mut engine = ExecutionEngine::interpreter(first).expect("an engine");
        let module = context.create_module("second");
        {
            let function = module.add_function("f", context.function_type(context.void_type(), &[], false));
            let _ = function.append_basic_block("entry");
        }
        engine.add_module(module);
    }
);

// ---- LTO ----

fn lto_module() -> lyftar::lto::LtoModule {
    let machine = TargetMachine::for_host().expect("a machine");
    let bitcode = with_context(|context| {
        let module = context.create_module("lto");
        module.set_target_machine(&machine);
        module.write_bitcode_to_memory_buffer().as_bytes().to_vec()
    });
    lyftar::lto::LtoModule::from_memory(&bitcode).expect("good bitcode")
}

#[test]
#[should_panic(expected = "symbol_name: index 0 is out of bounds for 0 symbols")]
fn decision_22_an_lto_symbol_name_out_of_bounds_panics() {
    let _ = lto_module().symbol_name(0);
}

#[test]
#[should_panic(expected = "symbol_attributes: index 0 is out of bounds for 0 symbols")]
fn decision_22_lto_symbol_attributes_out_of_bounds_panics() {
    let _ = lto_module().symbol_attributes(0);
}

#[test]
#[should_panic(expected = "set_cache_pruning_interval_in_seconds: 4294967295 is too long")]
fn decision_3_an_lto_interval_too_long_panics() {
    lyftar::lto::ThinLtoCodeGenerator::new().set_cache_pruning_interval_in_seconds(u32::MAX);
}

// ---- Passes: the panics a pass can hit on the way, which abort, are in `decision_3_pass_aborts` ----

panics!(decision_3_cloning_a_declaration_panics, "clone_function: g has no body", |context| {
    let module = context.create_module("m");
    let function = module.add_function("g", context.function_type(context.void_type(), &[], false));
    let _ = function.clone_function();
});
panics!(
    decision_3_passes_on_a_function_that_isnt_there_panics,
    "run_passes_on_function: this module has no function nope",
    |context| {
        let mut module = context.create_module("m");
        let _ = module.run_passes_on_function("nope", "instcombine", None, &PassOptions::default());
    }
);

// ---- Target data ----

#[test]
#[should_panic(expected = "size_of_type_in_bits: void has no size")]
fn decision_3_the_size_of_something_unsized_in_a_layout_panics() {
    let layout = TargetData::new("e").expect("a layout");
    with_context(|context| {
        let _ = layout.size_of_type_in_bits(context.void_type());
    });
}

#[test]
#[should_panic(expected = "element_at_offset: i64 is not a sized struct")]
fn decision_3_an_offset_into_something_not_a_struct_panics() {
    let layout = TargetData::new("e").expect("a layout");
    with_context(|context| {
        let _ = layout.element_at_offset(context.get_type::<I64>().to_dynamic(), 0);
    });
}

#[test]
#[should_panic(expected = "offset_of_element_in_bytes: { i64 } has no element 1")]
fn decision_22_an_element_offset_out_of_bounds_panics() {
    let layout = TargetData::new("e").expect("a layout");
    with_context(|context| {
        let pair = context.struct_type(&[context.get_type::<I64>().to_dynamic()], false);
        let _ = layout.offset_of_element_in_bytes(pair, 1);
    });
}

// The checks with no test here, each for why.
//
// Lengths past what LLVM's `unsigned` or `int` can hold need a list of more than two billion items, which is impossible to
// build in a test: `{operation}: {length} is more than LLVM can take`, `{operation}: too many items`, `parse_command_line_options:
// too many options`, `add_module: {} bytes is more than libLTO can take`, `{operation}: {} bytes is more than libLTO can take`,
// `contents: this section doesn't fit in memory`.
const CAN_NOT_BE_BUILT_IN_A_TEST: &[&str] = &[
    " is more than LLVM can take",
    ": too many items",
    "parse_command_line_options: too many options",
    " bytes is more than libLTO can take",
    "contents: this section doesn't fit in memory",
];

/// The pieces of a message's text with no `{...}` in them, long enough to say something.
fn literal_pieces(message: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    for character in message.chars() {
        match character {
            '{' => {
                depth += 1;
                pieces.push(std::mem::take(&mut current));
            }
            '}' => depth -= 1,
            _ if depth == 0 => current.push(character),
            _ => {}
        }
    }
    pieces.push(current);
    pieces.iter().map(|piece| piece.trim().to_string()).filter(|piece| piece.len() >= 8).collect()
}

#[test]
fn decision_3_every_message_in_the_crate_has_a_test() {
    // What aborts a pass runs in a child process, in the other file.
    let tests = concat!(include_str!("decision_3_panics.rs"), include_str!("decision_3_pass_aborts.rs"));
    let source_directory = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(source_directory).expect("the source") {
        let path = entry.expect("the source").path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("the source");
        for piece in source.split("bad_call(format_args!(").skip(1) {
            let Some(start) = piece.find('"') else { continue };
            let rest = &piece[start + 1..];
            let mut end = 0;
            let mut escaped = false;
            for (index, character) in rest.char_indices() {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == '"' {
                    end = index;
                    break;
                }
            }
            let message = rest[..end].replace("\\n", "");
            let joined: String = message.split_whitespace().collect::<Vec<_>>().join(" ");
            let pieces = literal_pieces(&joined);
            // The start of a piece is enough, since a test's expected text can stop short of a long message's end.
            let mentioned = |text: &str| pieces.iter().any(|piece| text.contains(&piece[..piece.len().min(24)]));
            if pieces.is_empty() || mentioned(tests) || CAN_NOT_BE_BUILT_IN_A_TEST.iter().any(|known| mentioned(known))
            {
                continue;
            }
            missing.push(format!("{}: {joined}", path.display()));
        }
    }
    assert!(missing.is_empty(), "these messages have no test:\n{}", missing.join("\n"));
}
