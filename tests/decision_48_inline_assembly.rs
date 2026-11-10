//! `builder.call` takes inline assembly as it takes a function, checked against the function type the assembly carries (D48).

use lyftar::enums::InlineAssemblyDialect;
use lyftar::{Bits, Builder, Integer, Value, with_context};

type I64 = Integer<Bits<64>>;

#[test]
fn decision_48_inline_assembly_is_called_with_a_tuple_a_slice_or_an_array() {
    with_context(|context| {
        let owned = context.create_module("assembly");
        let module = context.module(&owned);
        let i64_type = context.get_type::<I64>();
        let i64_dynamic = i64_type.to_dynamic();
        let function = module.add_function("f", context.function_type(i64_dynamic, &[i64_dynamic], false));
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let identity = context.function_type(i64_dynamic, &[i64_dynamic], false);
        let assembly = identity.inline_assembly("", "=r,0", false, false, InlineAssemblyDialect::Att, false);
        let parameter = function.parameter(0).checked_cast::<I64>();
        let from_tuple = builder.call(assembly, (parameter,), "from_tuple");
        let from_slice = builder.call(assembly, &[from_tuple.to_dynamic()][..], "from_slice");
        let from_array = builder.call(assembly, &[from_slice.to_dynamic()], "from_array");
        let from_vector = builder.call(assembly, &vec![from_array.to_dynamic()], "from_vector");
        let nothing = context.function_type(context.void_type(), &[], false);
        let side_effect = nothing.inline_assembly("", "", true, false, InlineAssemblyDialect::Att, false);
        builder.call(side_effect, &[] as &[Value<'_, '_>], "");
        builder.return_value(from_vector);
        drop(builder);
        assert_eq!(module.verify(), Ok(()), "{}", module.print_to_string());
        assert_eq!(module.print_to_string().matches("asm \"\", \"=r,0\"").count(), 4);
    });
}

#[test]
#[should_panic(expected = "call: ")]
fn decision_48_inline_assembly_with_the_wrong_arguments_panics() {
    with_context(|context| {
        let owned = context.create_module("assembly");
        let module = context.module(&owned);
        let i64_dynamic = context.get_type::<I64>().to_dynamic();
        let function = module.add_function("f", context.function_type(context.void_type(), &[], false));
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let identity = context.function_type(i64_dynamic, &[i64_dynamic], false);
        let assembly = identity.inline_assembly("", "=r,0", false, false, InlineAssemblyDialect::Att, false);
        builder.call(assembly, (), "");
    });
}
