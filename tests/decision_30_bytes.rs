//! LLVM 23's byte type, spelled `Byte<Bits<N>>` (D30), against real LLVM.

use lyftar::values::ConstantByte;
use lyftar::{Bits, Builder, Byte, Integer, Pointer, Type, with_context};

type B8 = Byte<Bits<8>>;
type B128 = Byte<Bits<128>>;

#[test]
fn decision_30_a_specific_byte_has_its_width_and_prints_as_llvm_says() {
    with_context(|context| {
        let byte = context.get_type::<B8>();
        assert_eq!(byte.width_in_bits(), 8);
        assert_eq!(byte.print_to_string(), "b8");
        assert_eq!(context.get_type::<B128>().print_to_string(), "b128");
        assert_eq!(context.get_type::<Byte<Bits<1>>>().width_in_bits(), 1);
        assert_eq!(context.get_type::<Byte<Bits<8_388_608>>>().width_in_bits(), 8_388_608);
    });
}

#[test]
fn decision_30_specific_dynamic_and_unknown_are_the_one_type() {
    with_context(|context| {
        let specific = context.get_type::<B8>();
        let dynamic_width = context.byte_type(8);
        let unknown: Type<'_> = specific.to_dynamic();
        assert_eq!(specific, dynamic_width);
        assert_eq!(specific, unknown);
        assert_eq!(unknown.checked_cast::<B8>(), specific);
        assert_eq!(dynamic_width.checked_cast::<B8>(), specific);
        assert_eq!(unknown.checked_cast::<Byte>().width_in_bits(), 8);
        // A byte isn't an integer of that width, and a byte of another width isn't this one.
        assert!(unknown.as_kind::<Integer<Bits<8>>>().is_none());
        assert!(unknown.as_kind::<Integer>().is_none());
        assert!(unknown.as_kind::<B128>().is_none());
        assert!(context.get_type::<Integer<Bits<8>>>().to_dynamic().as_kind::<Byte>().is_none());
    });
}

#[test]
fn decision_30_constants_keep_their_specific_type() {
    with_context(|context| {
        let byte = context.get_type::<B8>();
        let constant = byte.constant_byte(7);
        assert_eq!(constant.get_type(), byte);
        assert_eq!(constant.checked_cast::<ConstantByte>().zero_extended_value(), 7);
        assert_eq!(byte.constant_byte_from_string("-1", 10).checked_cast::<ConstantByte>().sign_extended_value(), -1);
        let wide = context.get_type::<B128>().constant_byte_from_words(&[1, 2]);
        assert_eq!(wide.print_to_string(), "b128 36893488147419103233");
        assert_eq!(context.byte_type(16).constant_byte(300).checked_cast::<ConstantByte>().zero_extended_value(), 300);
    });
}

#[test]
fn decision_30_a_byte_goes_through_memory_phis_and_selects_and_verifies() {
    with_context(|context| {
        let module = context.create_module("bytes");
        let byte = context.get_type::<B8>();
        let pointer = context.pointer_type(0).to_dynamic();
        let condition_type = context.get_type::<Integer<Bits<1>>>();
        let function_type = context.function_type(byte.to_dynamic(), &[pointer, condition_type.to_dynamic()], false);
        let function = module.add_function("copy", function_type);
        let entry = function.append_block("entry");
        let left = function.append_block("left");
        let right = function.append_block("right");
        let join = function.append_block("join");
        let builder = Builder::at_end_of(entry);
        let address = function.parameter(0).checked_cast::<Pointer>();
        let condition = function.parameter(1).checked_cast::<Integer<Bits<1>>>();
        let loaded = builder.load(byte, address, "loaded");
        let chosen = builder.select(condition, loaded, byte.constant_byte(1), "chosen");
        builder.store(chosen, address);
        builder.conditional_branch(condition, left, right);
        builder.position_at_end(left);
        builder.branch(join);
        builder.position_at_end(right);
        builder.branch(join);
        builder.position_at_end(join);
        let merged = builder.phi(byte, "merged");
        merged.add_incoming(&[(loaded, left), (chosen, right)]);
        builder.return_value(merged.as_value());
        assert_eq!(module.verify(), Ok(()));
        let text = module.print_to_string();
        assert!(text.contains("load b8, ptr %0"), "{text}");
        assert!(text.contains("phi b8"), "{text}");
    });
}

#[test]
#[should_panic(expected = "checked_cast: b8 is not a lyftar::types::Byte<lyftar::types::Bits<128>>")]
fn decision_30_casting_a_byte_to_another_width_panics() {
    with_context(|context| {
        let _ = context.get_type::<B8>().to_dynamic().checked_cast::<B128>();
    });
}
