//! The first thin slice through real LLVM.

use lyftar::{Bits, Builder, Integer, with_context};

#[test]
fn decision_4_a_specific_add_verifies_and_prints() {
    let printed = with_context(|context| {
        let module = context.create_module("slice");
        let i64_type = context.get_type::<Integer<Bits<64>>>();
        let function_type =
            context.function_type(i64_type.to_dynamic(), &[i64_type.to_dynamic(), i64_type.to_dynamic()], false);
        let function = module.add_function("add", function_type);
        let builder = Builder::at_end_of(function.append_block("entry"));
        let left = function.parameter(0).checked_cast::<Integer<Bits<64>>>();
        let right = function.parameter(1).checked_cast::<Integer<Bits<64>>>();
        let sum = builder.integer_add(left, right, "sum");
        builder.return_value(sum);
        module.verify().map_err(|report| report).map(|()| module.print_to_string())
    });
    assert!(printed.is_ok_and(|text| text.contains("%sum = add i64 %0, %1")));
}
