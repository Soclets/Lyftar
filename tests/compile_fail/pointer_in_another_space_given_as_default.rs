#![allow(unused_imports)]

use lyftar::enums::*;
use lyftar::passes::*;
use lyftar::target::*;
use lyftar::values::*;
use lyftar::*;

type Space5 = Pointer<AddressSpace<5>>;

fn main() {
    with_context(|context| {
        let module = context.create_module("m");
        let default_type = context.get_type::<Pointer>();
        let space_5_type = context.get_type::<Space5>();
        let function = module.add_function("f", context.function_type(context.void_type(), &[], false));
        let builder = Builder::at_end_of(function.append_block("entry"));
        let condition = context.get_type::<Integer<Bits<1>>>().constant_integer(1, false);
        let held: Value<'_, '_, Pointer> = space_5_type.constant_null();
        let _ = (held, default_type, builder, condition);
    });
}
