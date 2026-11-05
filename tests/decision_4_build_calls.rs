//! Build calls against real LLVM: everything built here goes through LLVM's verifier (rule 4).

use lyftar::enums::{AtomicOrdering, AtomicReadModifyWriteOperation, FloatPredicate, IntegerPredicate, Linkage};
use lyftar::values::{GlobalVariable, LoadInstruction};
use lyftar::{Bits, Builder, Float, Ieee754Binary, Integer, Pointer, with_context};

type I1 = Integer<Bits<1>>;
type I8 = Integer<Bits<8>>;
type I32 = Integer<Bits<32>>;
type I64 = Integer<Bits<64>>;
type F64 = Float<Ieee754Binary<Bits<64>>>;

#[test]
fn decision_4_specific_integer_arithmetic_verifies() {
    with_context(|context| {
        let module = context.create_module("arithmetic");
        let i64_type = context.get_type::<I64>();
        let function_type =
            context.function_type(i64_type.to_dynamic(), &[i64_type.to_dynamic(), i64_type.to_dynamic()], false);
        let function = module.add_function("arithmetic", function_type);
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let left = function.parameter(0).checked_cast::<I64>();
        let right = function.parameter(1).checked_cast::<I64>();
        let mut value = builder.integer_add(left, right, "add");
        value = builder.integer_add_no_signed_wrap(value, right, "");
        value = builder.integer_add_no_unsigned_wrap(value, right, "");
        value = builder.integer_subtract(value, right, "");
        value = builder.integer_subtract_no_signed_wrap(value, right, "");
        value = builder.integer_subtract_no_unsigned_wrap(value, right, "");
        value = builder.integer_multiply(value, right, "");
        value = builder.integer_multiply_no_signed_wrap(value, right, "");
        value = builder.integer_multiply_no_unsigned_wrap(value, right, "");
        value = builder.unsigned_divide(value, right, "");
        value = builder.exact_unsigned_divide(value, right, "");
        value = builder.signed_divide(value, right, "");
        value = builder.exact_signed_divide(value, right, "");
        value = builder.unsigned_remainder(value, right, "");
        value = builder.signed_remainder(value, right, "");
        value = builder.shift_left(value, right, "");
        value = builder.logical_shift_right(value, right, "");
        value = builder.arithmetic_shift_right(value, right, "");
        value = builder.and(value, right, "");
        value = builder.or(value, right, "");
        value = builder.exclusive_or(value, right, "");
        value = builder.integer_negate(value, "");
        value = builder.integer_negate_no_signed_wrap(value, "");
        value = builder.integer_negate_no_unsigned_wrap(value, "");
        value = builder.not(value, "named");
        assert_eq!(value.name(), "named");
        builder.return_value(value);
        assert_eq!(module.verify(), Ok(()));
    });
}

#[test]
fn decision_4_dynamic_float_compare_cast_and_control_flow_verify() {
    with_context(|context| {
        let module = context.create_module("flow");
        let f64_type = context.get_type::<F64>();
        let i32_type = context.get_type::<I32>();
        let i64_type = context.get_type::<I64>();
        let function_type = context.function_type(i64_type.to_dynamic(), &[f64_type.to_dynamic()], false);
        let function = module.add_function("flow", function_type);
        let entry = function.append_basic_block("entry");
        let then_block = function.append_basic_block("then");
        let else_block = function.append_basic_block("else");
        let join = function.append_basic_block("join");
        let builder = Builder::at_end_of(entry);
        // A value from the program being compiled is dynamic, checked when the line runs.
        let parameter = function.parameter(0);
        let doubled = builder.float_add(parameter, parameter, "doubled");
        let specific = doubled.checked_cast::<F64>();
        let zero = f64_type.constant_float(0.0);
        let test = builder.float_compare(FloatPredicate::OrderedGreaterThan, specific, zero, "test");
        let condition: lyftar::Value<'_, '_, I1> = test;
        builder.conditional_branch(condition, then_block, else_block);

        builder.position_at_end(then_block);
        let as_32 = builder.float_to_signed_integer(specific, i32_type, "narrow");
        let extended = builder.sign_extend(as_32, i64_type, "wide");
        builder.branch(join);

        builder.position_at_end(else_block);
        let as_unsigned = builder.float_to_unsigned_integer(specific, i64_type, "");
        builder.branch(join);

        builder.position_at_end(join);
        let phi = builder.phi(i64_type, "result");
        phi.add_incoming(&[(extended, then_block), (as_unsigned, else_block)]);
        assert_eq!(phi.incoming_count(), 2);
        assert_eq!(phi.incoming_basic_block(1), else_block);
        let compared = builder.integer_compare(IntegerPredicate::SignedLessThan, phi.as_value(), phi.as_value(), "");
        let chosen = builder.select(compared, phi.as_value(), phi.as_value(), "chosen");
        builder.return_value(chosen);
        assert_eq!(module.verify(), Ok(()));
    });
}

#[test]
fn decision_4_memory_calls_and_atomics_verify() {
    with_context(|context| {
        let module = context.create_module("memory");
        let i8_type = context.get_type::<I8>();
        let i64_type = context.get_type::<I64>();
        let pointer_type = context.pointer_type(0);
        let callee_type = context.function_type(i64_type.to_dynamic(), &[pointer_type.to_dynamic()], false);
        let callee = module.add_function("callee", callee_type);
        let function_type = context.function_type(context.void_type(), &[], false);
        let function = module.add_function("memory", function_type);
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let struct_type = context.struct_type(&[i64_type.to_dynamic(), i8_type.to_dynamic()], false);
        let slot = builder.stack_allocate(struct_type, "slot");
        slot.set_alignment_in_bytes(16);
        assert_eq!(slot.alignment_in_bytes(), 16);
        let field = builder.struct_get_element_pointer(struct_type, slot, 1, "field");
        builder.store(i8_type.constant_integer(7, false), field);
        let loaded = builder.load(i8_type, field, "loaded");
        loaded.checked_cast::<LoadInstruction>().set_volatile(true);
        let index = i64_type.constant_integer(0, false);
        let field_index = context.get_type::<I32>().constant_integer(0, false);
        let first =
            builder.get_element_pointer(struct_type, slot, &[index.to_dynamic(), field_index.to_dynamic()], "first");
        let _ = builder.call(callee, &[first.to_dynamic()], "called");
        builder.fence(AtomicOrdering::SequentiallyConsistent, false);
        builder.atomic_read_modify_write(
            AtomicReadModifyWriteOperation::Add,
            first,
            index,
            AtomicOrdering::Monotonic,
            false,
        );
        builder.atomic_compare_exchange(
            first,
            index,
            index,
            AtomicOrdering::Monotonic,
            AtomicOrdering::Monotonic,
            false,
        );
        let length = i64_type.constant_integer(8, false);
        builder.memory_set(first, i8_type.constant_integer(0, false), length, 8);
        builder.memory_copy(first, 8, slot, 8, length);
        builder.return_void();
        assert_eq!(module.verify(), Ok(()));
    });
}

#[test]
fn decision_4_globals_and_aggregates_verify() {
    with_context(|context| {
        let module = context.create_module("globals");
        let i32_type = context.get_type::<I32>();
        let array = i32_type.array_type(2);
        let two = [i32_type.constant_integer(1, false), i32_type.constant_integer(2, false)];
        let initializer = i32_type.constant_array(&two);
        let global = module.add_global("table", array);
        global.set_initializer(initializer);
        global.set_linkage(Linkage::Private);
        global.set_global_constant(true);
        assert_eq!(module.get_global("table"), Some(global));
        assert!(module.global_variables().any(|found| found == global));
        let function_type = context.function_type(i32_type.to_dynamic(), &[], false);
        let function = module.add_function("read", function_type);
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let aggregate = builder.load(array, global, "aggregate");
        let element = builder.extract_value(aggregate, 1, "element");
        builder.return_value(element);
        let text = module.print_to_string();
        assert!(text.contains("private constant [2 x i32] [i32 1, i32 2]"), "{text}");
        assert_eq!(module.verify(), Ok(()));
        let _: Option<lyftar::Value<'_, '_, GlobalVariable>> = global.next_global();
        // Its address space is only known when the line runs (D31).
        let _: lyftar::Value<'_, '_> = global.as_pointer();
        let _: lyftar::Value<'_, '_, Pointer> = global.as_pointer().checked_cast::<Pointer>();
    });
}
