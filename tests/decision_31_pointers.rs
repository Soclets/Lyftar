//! Pointers and their address spaces (D31), against real LLVM.

use lyftar::{AddressSpace, Bits, Builder, Integer, Pointer, Type, Value, with_context};

type I64 = Integer<Bits<64>>;
type Space5 = Pointer<AddressSpace<5>>;

#[test]
fn decision_31_plain_pointer_is_address_space_zero() {
    with_context(|context| {
        let plain = context.get_type::<Pointer>();
        let spelled: Type<'_, Pointer<AddressSpace<0>>> = plain;
        assert_eq!(spelled, plain);
        assert_eq!(plain, context.pointer_type(0));
        assert_eq!(plain.address_space(), 0);
        assert_eq!(plain.print_to_string(), "ptr");
    });
}

#[test]
fn decision_31_another_space_is_its_own_type() {
    with_context(|context| {
        let space_5 = context.get_type::<Space5>();
        assert_eq!(space_5, context.pointer_type(5));
        assert_eq!(space_5.address_space(), 5);
        assert_eq!(space_5.print_to_string(), "ptr addrspace(5)");
        assert_ne!(space_5, context.get_type::<Pointer>());
        assert_eq!(context.get_type::<Pointer<AddressSpace<16_777_215>>>().address_space(), 16_777_215);
    });
}

#[test]
fn decision_31_a_pointer_of_unknown_space_is_dynamic_and_casts_check_the_space() {
    with_context(|context| {
        let unknown: Type<'_> = context.pointer_type(5);
        assert_eq!(unknown.address_space(), 5);
        assert_eq!(unknown.checked_cast::<Space5>(), context.get_type::<Space5>());
        assert!(unknown.as_kind::<Pointer>().is_none());
        assert!(context.get_type::<I64>().to_dynamic().as_kind::<Space5>().is_none());
        let null: Value<'_, '_> = unknown.constant_null();
        assert!(null.as_kind::<Space5>().is_some());
        assert!(null.as_kind::<Pointer>().is_none());
    });
}

#[test]
fn decision_31_a_global_in_another_space_is_not_a_plain_pointer() {
    with_context(|context| {
        let module = context.create_module("spaces");
        let i64_type = context.get_type::<I64>();
        let global = module.add_global_in_address_space("in_space_5", i64_type, 5);
        assert_eq!(global.get_type().address_space(), 5);
        assert!(global.as_pointer().as_kind::<Pointer>().is_none());
        assert!(global.as_pointer().as_kind::<Space5>().is_some());
    });
}

#[test]
fn decision_31_memory_works_in_any_space_and_a_get_element_pointer_keeps_its_space() {
    with_context(|context| {
        let module = context.create_module("spaces");
        let i64_type = context.get_type::<I64>();
        let space_5 = context.get_type::<Space5>().to_dynamic();
        let function_type = context.function_type(i64_type.to_dynamic(), &[space_5], false);
        let function = module.add_function("f", function_type);
        let builder = Builder::at_end_of(function.append_block("entry"));
        let address = function.parameter(0).checked_cast::<Space5>();
        let index = i64_type.constant_integer(1, false);
        let next: Value<'_, '_, Space5> = builder.get_element_pointer(i64_type.to_dynamic(), address, &[index], "next");
        let loaded = builder.load(i64_type, next, "loaded");
        builder.store(loaded, address);
        let plain = builder.address_space_cast(next, context.get_type::<Pointer>(), "plain");
        let again: Value<'_, '_, Space5> = builder.address_space_cast(plain, context.get_type::<Space5>(), "again");
        let compared = builder.pointer_compare(lyftar::enums::IntegerPredicate::Equal, address, again, "same");
        let _ = compared;
        builder.return_value(loaded);
        assert_eq!(module.verify(), Ok(()));
        let text = module.print_to_string();
        assert!(text.contains("getelementptr i64, ptr addrspace(5) %0"), "{text}");
        assert!(text.contains("addrspacecast ptr addrspace(5) %next to ptr"), "{text}");
    });
}

#[test]
fn decision_31_constants_cast_across_spaces() {
    with_context(|context| {
        let module = context.create_module("spaces");
        let i64_type = context.get_type::<I64>();
        let global = module.add_global_in_address_space("in_space_5", i64_type, 5);
        let cast: Value<'_, '_, Pointer> =
            global.as_pointer().constant_address_space_cast(context.get_type::<Pointer>());
        assert_eq!(cast.print_to_string(), "ptr addrspacecast (ptr addrspace(5) @in_space_5 to ptr)");
        let to_integer = cast.constant_pointer_to_integer(i64_type);
        let back: Value<'_, '_, Space5> = to_integer.constant_integer_to_pointer(context.get_type::<Space5>());
        assert_eq!(back.get_type().address_space(), 5);
        assert_eq!(module.verify(), Ok(()));
    });
}
