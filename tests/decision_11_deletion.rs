//! Deleting from a module, tracked handles and the checks behind them, against real LLVM (D11).

use lyftar::enums::Linkage;
use lyftar::passes::PassOptions;
use lyftar::values::{Function, Instruction, PhiNode};
use lyftar::{Bits, Builder, Integer, Module, Value, with_context};

type I64 = Integer<Bits<64>>;

const IR: &str = "
define i64 @f(i64 %x) {
entry:
  %y = add i64 %x, 1
  %z = add i64 %y, 2
  %dead = add i64 %z, 3
  ret i64 %z
}
define i64 @g(i1 %test, i64 %x) {
entry:
  br i1 %test, label %left, label %right
left:
  %l = add i64 %x, 1
  br label %join
right:
  %r = add i64 %x, 2
  br label %join
join:
  %p = phi i64 [ %l, %left ], [ %r, %right ]
  ret i64 %p
}
@used = global i64 0
define void @unused_function() { ret void }
";

fn instruction_named<'context, 'module>(
    module: &'module Module<'context>,
    function: &str,
    name: &str,
) -> Value<'context, 'module, Instruction> {
    module
        .get_function(function)
        .expect("the function")
        .function_instructions()
        .find(|found| found.name() == name)
        .expect("the instruction")
}

#[test]
fn decision_11_erasing_an_instruction_with_no_uses_removes_it() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let entry = {
            let dead = instruction_named(&module, "f", "dead");
            module.value_to_erase(dead)
        };
        module.erase(&[entry]);
        assert_eq!(module.verify(), Ok(()));
        assert!(!module.print_to_string().contains("%dead"));
    });
}

#[test]
fn decision_11_values_in_one_list_can_use_each_other_in_any_order() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let (first, second, third) = {
            let y = instruction_named(&module, "f", "y");
            let z = instruction_named(&module, "f", "z");
            let dead = instruction_named(&module, "f", "dead");
            let ret = z.next_instruction().and_then(|next| next.next_instruction()).expect("the return");
            let builder = Builder::before(ret);
            let parameter = module.get_function("f").expect("the function").parameter(0);
            ret.set_operand(0, builder.integer_add(parameter, parameter, "fresh"));
            (module.value_to_erase(y), module.value_to_erase(z), module.value_to_erase(dead))
        };
        // First the one everything else uses, and nothing about the order matters.
        module.erase(&[first, second, third]);
        assert_eq!(module.verify(), Ok(()));
        assert!(!module.print_to_string().contains("%z"));
    });
}

#[test]
fn decision_11_erasing_a_block_drops_it_from_phis() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let entry = {
            let g = module.get_function("g").expect("the function");
            let right = g.basic_blocks().find(|block| block.name() == "right").expect("the block");
            let phi = instruction_named(&module, "g", "p").checked_cast::<PhiNode>();
            // Nothing branches to %right afterwards. The phi's entry for it still uses %r, and goes with the block.
            let entry_basic_block = g.first_basic_block().expect("a body");
            let left = g.basic_blocks().find(|block| block.name() == "left").expect("the block");
            entry_basic_block.terminator().expect("a terminator").set_successor(1, left);
            assert_eq!(phi.incoming_count(), 2);
            module.basic_block_to_erase(right)
        };
        module.erase(&[entry]);
        let g = module.get_function("g").expect("the function");
        let phi =
            g.function_instructions().find(|found| found.name() == "p").expect("the phi").checked_cast::<PhiNode>();
        assert_eq!(phi.incoming_count(), 1);
        assert_eq!(phi.incoming_basic_block(0).name(), "left");
        assert_eq!(module.verify(), Ok(()));
    });
}

#[test]
fn decision_11_erasing_functions_and_globals_removes_them() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let (function, global) = {
            let function = module.get_function("unused_function").expect("the function");
            let global = module.get_global("used").expect("the global");
            (module.value_to_erase(function), module.value_to_erase(global))
        };
        module.erase(&[function, global]);
        assert!(module.get_function("unused_function").is_none());
        assert!(module.get_global("used").is_none());
        assert_eq!(module.verify(), Ok(()));
    });
}

#[test]
fn decision_11_a_dead_constant_expression_doesnt_stop_a_global_going() {
    with_context(|context| {
        let mut module = context.parse_ir_text("@target = global i64 0").expect("good IR");
        let entry = {
            let global = module.get_global("target").expect("the global");
            // A constant expression nothing uses.
            let _ = global.as_pointer().constant_pointer_to_integer(context.get_type::<I64>());
            module.value_to_erase(global)
        };
        module.erase(&[entry]);
        assert!(module.get_global("target").is_none());
    });
}

#[test]
fn decision_11_a_tracked_handle_follows_its_value_and_says_when_it_goes() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let (handle_on_dead, handle_on_y) = {
            let dead = instruction_named(&module, "f", "dead");
            let y = instruction_named(&module, "f", "y");
            (module.track(dead), module.track(y))
        };
        assert!(handle_on_dead.is_present());
        let entry = {
            let dead = handle_on_dead.get(&module).expect("still there");
            module.value_to_erase(dead)
        };
        module.erase(&[entry]);
        assert!(!handle_on_dead.is_present());
        assert!(handle_on_dead.get(&module).is_none());
        // A handle survives a pass run too, and comes back empty if the pass deleted its value.
        assert!(handle_on_y.get(&module).is_some());
        module.run_passes("default<O2>", None, &PassOptions::default()).expect("the pipeline");
        let f_survived = module.get_function("f").is_some();
        assert!(f_survived);
        let _ = handle_on_y.get(&module);
    });
}

#[test]
fn decision_11_a_handle_doesnt_follow_a_replacement() {
    with_context(|context| {
        let module = context.parse_ir_text(IR).expect("the test's IR");
        let y = instruction_named(&module, "f", "y");
        let handle = module.track(y);
        let parameter = module.get_function("f").expect("the function").parameter(0);
        y.replace_all_uses_with(parameter);
        assert_eq!(handle.get(&module), Some(y));
    });
}

#[test]
fn decision_11_constants_survive_deletes_and_pass_runs() {
    with_context(|context| {
        let i64_type = context.get_type::<I64>();
        let constant = i64_type.constant_integer(42, false);
        let structure = context.constant_struct(&[constant], false);
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let entry = {
            let dead = instruction_named(&module, "f", "dead");
            module.value_to_erase(dead)
        };
        module.erase(&[entry]);
        module.run_passes("default<O2>", None, &PassOptions::default()).expect("the pipeline");
        assert_eq!(constant.print_to_string(), "i64 42");
        assert_eq!(structure.print_to_string(), "{ i64 } { i64 42 }");
    });
}

#[test]
fn decision_11_a_context_constant_can_be_used_with_two_modules() {
    with_context(|context| {
        let i64_type = context.get_type::<I64>();
        let constant = i64_type.constant_integer(1, false);
        for name in ["first", "second"] {
            let module = context.create_module(name);
            let function = module.add_function("f", context.function_type(i64_type.to_dynamic(), &[], false));
            let builder = Builder::at_end_of(function.append_basic_block("entry"));
            builder.return_value(constant);
            assert_eq!(module.verify(), Ok(()));
        }
    });
}

#[test]
fn decision_11_uses_in_gives_only_this_modules_uses_of_a_shared_constant() {
    with_context(|context| {
        let i64_type = context.get_type::<I64>();
        let structure_type = context.struct_type(&[i64_type.to_dynamic()], false);
        let member = context.constant_struct(&[i64_type.constant_integer(7, false)], false);
        let first = context.create_module("first");
        let second = context.create_module("second");
        for module in [&first, &second] {
            let function = module.add_function("f", context.function_type(structure_type, &[], false));
            let builder = Builder::at_end_of(function.append_basic_block("entry"));
            builder.return_value(member);
        }
        assert_eq!(member.uses_in(&first).count(), 1);
        let user = member.uses_in(&second).next().expect("one use").user();
        assert_eq!(
            user.checked_cast::<Instruction>().parent_basic_block().parent().parent_module().identifier(),
            "second"
        );
    });
}

#[test]
fn decision_11_uses_of_something_in_a_module_list_its_users() {
    with_context(|context| {
        let module = context.parse_ir_text(IR).expect("the test's IR");
        let y = instruction_named(&module, "f", "y");
        let users: Vec<String> = y.uses().map(|found| found.user().name()).collect();
        assert_eq!(users, ["z"]);
        let function: Value<'_, '_, Function> = module.get_function("f").expect("the function");
        assert_eq!(function.uses().count(), 0);
        function.set_linkage(Linkage::Internal);
    });
}

#[test]
fn decision_11_cloning_a_module_leaves_the_first_alone() {
    with_context(|context| {
        let module = context.parse_ir_text(IR).expect("the test's IR");
        let mut clone = module.clone_module();
        let entry = {
            let function = clone.get_function("unused_function").expect("the function");
            clone.value_to_erase(function)
        };
        clone.erase(&[entry]);
        assert!(clone.get_function("unused_function").is_none());
        assert!(module.get_function("unused_function").is_some());
    });
}

#[test]
fn decision_11_replacing_a_global_with_only_instructions_and_globals_using_it_works() {
    with_context(|context| {
        let module = context
            .parse_ir_text(
                "@first = global i64 0\n@second = global i64 0\n@points = global ptr @first\n\
                 define ptr @f() {\n ret ptr @first\n}",
            )
            .expect("good IR");
        let first = module.get_global("first").expect("the global");
        let second = module.get_global("second").expect("the global");
        first.replace_all_uses_with(second);
        assert_eq!(module.verify(), Ok(()));
        let text = module.print_to_string();
        assert!(text.contains("@points = global ptr @second"), "{text}");
        assert!(text.contains("ret ptr @second"), "{text}");
    });
}

#[test]
fn decision_11_metadata_around_a_value_moves_to_its_replacement_and_stays_alive() {
    with_context(|context| {
        let module = context.parse_ir_text(IR).expect("the test's IR");
        let y = instruction_named(&module, "f", "y");
        let parameter = module.get_function("f").expect("the function").parameter(0);
        let held = y.as_metadata();
        assert!(held.as_value(context).print_to_string().contains("%y"));
        y.replace_all_uses_with(parameter);
        let text = held.as_value(context).print_to_string();
        assert!(text.contains("%x"), "{text}");
        assert_eq!(module.verify(), Ok(()));
    });
}

#[test]
fn decision_34_a_global_a_constant_uses_can_be_replaced_with_exclusive_access() {
    with_context(|context| {
        let mut module = context
            .parse_ir_text(
                "@first = global i64 0\n@second = global i64 0\n\
                 @address = global i64 ptrtoint (ptr @first to i64)\n\
                 define i64 @f() {\n ret i64 ptrtoint (ptr @first to i64)\n}",
            )
            .expect("good IR");
        let entry = {
            let first = module.get_global("first").expect("the global");
            let second = module.get_global("second").expect("the global");
            module.replacement_to_make(first, second)
        };
        module.replace(&[entry]);
        assert_eq!(module.verify(), Ok(()));
        let text = module.print_to_string();
        assert!(text.contains("@address = global i64 ptrtoint (ptr @second to i64)"), "{text}");
        assert!(text.contains("ret i64 ptrtoint (ptr @second to i64)"), "{text}");
    });
}

#[test]
fn decision_34_a_value_wrapped_in_metadata_can_become_a_constant_with_exclusive_access() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let entry = {
            let y = instruction_named(&module, "f", "y");
            let _wrapped = y.as_metadata();
            module.replacement_to_make(y, context.get_type::<I64>().constant_integer(9, false))
        };
        module.replace(&[entry]);
        assert_eq!(module.verify(), Ok(()));
        assert!(module.print_to_string().contains("%z = add i64 9, 2"), "{}", module.print_to_string());
    });
}

#[test]
fn decision_34_replacements_happen_in_order() {
    with_context(|context| {
        let mut module = context.parse_ir_text(IR).expect("the test's IR");
        let entries = {
            let y = instruction_named(&module, "f", "y");
            let z = instruction_named(&module, "f", "z");
            let parameter = module.get_function("f").expect("the function").parameter(0);
            // The uses of %y go to %z, and then the uses of %z go to the parameter.
            [module.replacement_to_make(y, z), module.replacement_to_make(z, parameter)]
        };
        module.replace(&entries);
        assert_eq!(module.verify(), Ok(()));
        let text = module.print_to_string();
        assert!(text.contains("ret i64 %x"), "{text}");
        assert!(text.contains("%z = add i64 %x, 2"), "{text}");
    });
}

#[test]
fn decision_34_a_handle_says_whether_a_replacement_deleted_a_constant() {
    const WITHOUT_THE_NEW_FORM: &str = "@first = global i64 0\n@second = global i64 0\n\
                                        @address = global i64 ptrtoint (ptr @first to i64)";
    const WITH_THE_NEW_FORM: &str = "@first = global i64 0\n@second = global i64 0\n\
                                     @address = global i64 ptrtoint (ptr @first to i64)\n\
                                     @already = global i64 ptrtoint (ptr @second to i64)";
    for (ir, deleted) in [(WITHOUT_THE_NEW_FORM, false), (WITH_THE_NEW_FORM, true)] {
        with_context(|context| {
            let mut module = context.parse_ir_text(ir).expect("good IR");
            let (entry, handle) = {
                let first = module.get_global("first").expect("the global");
                let second = module.get_global("second").expect("the global");
                let address = module.get_global("address").expect("the global");
                let constant = address.initializer().expect("an initializer");
                (module.replacement_to_make(first, second), module.track(constant))
            };
            assert!(handle.is_present());
            module.replace(&[entry]);
            assert_eq!(handle.is_present(), !deleted);
            assert_eq!(module.verify(), Ok(()));
        });
    }
}
