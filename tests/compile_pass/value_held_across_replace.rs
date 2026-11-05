#![allow(unused_imports)]

use lyftar::enums::*;
use lyftar::passes::*;
use lyftar::target::*;
use lyftar::values::*;
use lyftar::*;

const IR: &str = "define i64 @f(i64 %x) {\n %y = add i64 %x, 1\n ret i64 %y\n}";

fn main() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("good IR");
        let function = context.module(&module).get_function("f").expect("the function");
        let parameter = function.parameter(0);
        let y = function.function_instructions().next().expect("the add");
        let entry = context.module(&module).replacement_to_make(y, parameter);
        let _ = y.name();
        context.exclusive_module(&mut module).replace(&[entry]);
    });
}
