//! Specific arrays, vectors and structs (D32), against real LLVM.

use lyftar::enums::IntegerPredicate;
use lyftar::{
    AddressSpace, Array, Bits, Builder, Byte, Elements, Float, Ieee754Binary, Integer, MinimumElements, PackedStruct,
    Pointer, ScalableVector, Struct, Type, Vector, with_context,
};

type I1 = Integer<Bits<1>>;
type I32 = Integer<Bits<32>>;
type F64 = Float<Ieee754Binary<Bits<64>>>;

#[test]
fn decision_32_each_spelling_is_the_type_llvm_prints() {
    with_context(|context| {
        assert_eq!(context.get_type::<Array<I32, Elements<4>>>().print_to_string(), "[4 x i32]");
        assert_eq!(context.get_type::<Array<I32, Elements<0>>>().print_to_string(), "[0 x i32]");
        assert_eq!(context.get_type::<Vector<I32, Elements<4>>>().print_to_string(), "<4 x i32>");
        assert_eq!(
            context.get_type::<ScalableVector<I32, MinimumElements<4>>>().print_to_string(),
            "<vscale x 4 x i32>"
        );
        assert_eq!(context.get_type::<Struct<(I32, Pointer)>>().print_to_string(), "{ i32, ptr }");
        assert_eq!(context.get_type::<PackedStruct<(I32, I32)>>().print_to_string(), "<{ i32, i32 }>");
        assert_eq!(context.get_type::<Struct<()>>().print_to_string(), "{}");
        assert_eq!(
            context.get_type::<Struct<(Array<Vector<F64, Elements<2>>, Elements<3>>, Struct<(Pointer<AddressSpace<5>>,)>)>>().print_to_string(),
            "{ [3 x <2 x double>], { ptr addrspace(5) } }"
        );
        assert_eq!(context.get_type::<Vector<Byte<Bits<8>>, Elements<4>>>().print_to_string(), "<4 x b8>");
        assert_eq!(context.get_type::<Vector<Pointer, Elements<2>>>().print_to_string(), "<2 x ptr>");
        assert_eq!(
            context.get_type::<Struct<(I1, I1, I1, I1, I1, I1, I1, I1, I1, I1, I1, I32)>>().print_to_string(),
            "{ i1, i1, i1, i1, i1, i1, i1, i1, i1, i1, i1, i32 }"
        );
    });
}

#[test]
fn decision_32_a_spelling_is_the_same_type_as_the_dynamic_form() {
    with_context(|context| {
        let i32_type = context.get_type::<I32>();
        assert_eq!(context.get_type::<Array<I32, Elements<4>>>(), i32_type.array_type(4));
        assert_eq!(context.get_type::<Vector<I32, Elements<4>>>(), i32_type.vector_type(4));
        assert_eq!(context.get_type::<ScalableVector<I32, MinimumElements<4>>>(), i32_type.scalable_vector_type(4));
        let pointer = context.get_type::<Pointer>().to_dynamic();
        assert_eq!(
            context.get_type::<Struct<(I32, Pointer)>>(),
            context.struct_type(&[i32_type.to_dynamic(), pointer], false)
        );
        let both = [i32_type.to_dynamic(), i32_type.to_dynamic()];
        assert_eq!(context.get_type::<PackedStruct<(I32, I32)>>(), context.struct_type(&both, true));
    });
}

#[test]
fn decision_32_casts_tell_one_shape_from_another() {
    with_context(|context| {
        let i32_type = context.get_type::<I32>();
        let array: Type<'_> = i32_type.array_type(4);
        assert_eq!(array.checked_cast::<Array<I32, Elements<4>>>().print_to_string(), "[4 x i32]");
        assert!(array.as_kind::<Array<I32, Elements<5>>>().is_none());
        assert!(array.as_kind::<Array<Integer<Bits<64>>, Elements<4>>>().is_none());
        assert!(array.as_kind::<Vector<I32, Elements<4>>>().is_none());
        let plain = context.struct_type(&[i32_type.to_dynamic()], false);
        assert!(plain.as_kind::<Struct<(I32,)>>().is_some());
        assert!(plain.as_kind::<PackedStruct<(I32,)>>().is_none());
        // A named struct stays dynamic, even with the same body.
        let named = context.create_named_struct("named");
        named.set_struct_body(&[i32_type.to_dynamic()], false);
        assert!(named.as_kind::<Struct<(I32,)>>().is_none());
    });
}

#[test]
fn decision_32_constants_and_memory_keep_their_specific_type() {
    with_context(|context| {
        type Pair = Struct<(I32, I32)>;
        let module = context.create_module("aggregates");
        let pair = context.get_type::<Pair>();
        let function_type =
            context.function_type(pair.to_dynamic(), &[context.get_type::<Pointer>().to_dynamic()], false);
        let function = module.add_function("f", function_type);
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let address = function.parameter(0).checked_cast::<Pointer>();
        let loaded = builder.load(pair, address, "loaded");
        builder.store(pair.constant_null(), address);
        builder.store(loaded, address);
        builder.return_value(loaded);
        assert_eq!(module.verify(), Ok(()));
        let i32_type = context.get_type::<I32>();
        let one = i32_type.constant_integer(1, false);
        let constant = i32_type.constant_array(&[one, one]).checked_cast::<Array<I32, Elements<2>>>();
        assert_eq!(constant.print_to_string(), "[2 x i32] [i32 1, i32 1]");
    });
}

#[test]
fn decision_32_vectors_are_operands_with_no_check_left_to_run() {
    with_context(|context| {
        type Four = Vector<I32, Elements<4>>;
        type FourFloats = Vector<F64, Elements<4>>;
        let module = context.create_module("vectors");
        let four = context.get_type::<Four>();
        let floats = context.get_type::<FourFloats>();
        let parameters = [four.to_dynamic(), floats.to_dynamic()];
        let function = module.add_function("f", context.function_type(four.to_dynamic(), &parameters, false));
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let left = function.parameter(0).checked_cast::<Four>();
        let float = function.parameter(1).checked_cast::<FourFloats>();
        let sum = builder.integer_add(left, left, "sum");
        let compared = builder.integer_compare(IntegerPredicate::Equal, sum, left, "compared");
        let _: lyftar::Value<'_, '_, Vector<I1, Elements<4>>> = compared;
        let chosen = builder.select(compared, sum, left, "chosen");
        let doubled = builder.float_add(float, float, "doubled");
        let _ = doubled;
        builder.return_value(chosen);
        assert_eq!(module.verify(), Ok(()));
        let text = module.print_to_string();
        assert!(text.contains("add <4 x i32> %0, %0"), "{text}");
        assert!(text.contains("select <4 x i1> %compared"), "{text}");
    });
}

#[test]
fn decision_32_scalable_vectors_are_operands_too() {
    with_context(|context| {
        type Scaled = ScalableVector<I32, MinimumElements<4>>;
        let module = context.create_module("vectors");
        let scaled = context.get_type::<Scaled>();
        let function =
            module.add_function("f", context.function_type(scaled.to_dynamic(), &[scaled.to_dynamic()], false));
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let value = function.parameter(0).checked_cast::<Scaled>();
        let sum = builder.integer_add(value, value, "sum");
        let compared = builder.integer_compare(IntegerPredicate::Equal, sum, value, "compared");
        let _: lyftar::Value<'_, '_, ScalableVector<I1, MinimumElements<4>>> = compared;
        builder.return_value(sum);
        assert_eq!(module.verify(), Ok(()));
    });
}

#[test]
#[should_panic(expected = "load: { <vscale x 4 x i32>, i32 } can't be loaded or stored")]
fn decision_32_a_struct_with_no_known_size_still_cant_be_loaded() {
    with_context(|context| {
        type NoSize = Struct<(ScalableVector<I32, MinimumElements<4>>, I32)>;
        let module = context.create_module("m");
        let pointer = context.get_type::<Pointer>();
        let function =
            module.add_function("f", context.function_type(context.void_type(), &[pointer.to_dynamic()], false));
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let address = function.parameter(0).checked_cast::<Pointer>();
        let _ = builder.load(context.get_type::<NoSize>(), address, "");
    });
}

#[test]
#[should_panic(
    expected = "checked_cast: [4 x i32] is not a lyftar::types::Array<lyftar::types::Integer<lyftar::types::Bits<32>>, lyftar::types::Elements<5>>"
)]
fn decision_32_casting_to_another_length_panics() {
    with_context(|context| {
        let _ = context.get_type::<I32>().array_type(4).checked_cast::<Array<I32, Elements<5>>>();
    });
}
