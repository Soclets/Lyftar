//! Modules live inside the context, and every value borrows it (D42, D43). What this rules out doesn't compile, and is in
//! `tests/compile_fail`. These are what it still allows, against real LLVM.

use lyftar::enums::Linkage;
use lyftar::jit::ExecutionEngine;
use lyftar::module::Module;
use lyftar::passes::{Analyses, ModulePass, PassOptions, PreservedAnalyses};
use lyftar::{Bits, Builder, Integer, with_context};

type I32 = Integer<Bits<32>>;

#[test]
fn decision_43_a_constant_can_be_shared_across_modules() {
    with_context(|context| {
        let one = context.get_type::<I32>().constant_integer(1, false);
        let first = context.create_module("first");
        let second = context.create_module("second");
        let shared = context.constant_struct(&[one, one], false);
        let in_first = context.module(&first).add_global("g", shared.get_type());
        let in_second = context.module(&second).add_global("g", shared.get_type());
        in_first.set_initializer(shared);
        in_second.set_initializer(shared);
        // LLVM keeps one copy, which is why a pass on either has to need the whole context.
        assert_eq!(in_first.initializer(), in_second.initializer());
        assert_eq!(context.module(&first).verify(), Ok(()));
        assert_eq!(context.module(&second).verify(), Ok(()));
    });
}

// The shape of the use-after-free, with nothing held: the constant really is deleted, and asking for it again makes a new one.
#[test]
fn decision_43_a_pass_on_one_module_can_delete_what_another_used_and_a_new_one_can_be_made() {
    with_context(|context| {
        let one = context.get_type::<I32>().constant_integer(1, false);
        let mut first = context.create_module("first");
        let second = context.create_module("second");
        {
            let shared = context.constant_struct(&[one, one], false);
            let dead = context.module(&first).add_global("dead", shared.get_type());
            dead.set_initializer(shared);
            dead.set_linkage(Linkage::Internal);
            let also = context.module(&second).add_global("also", shared.get_type());
            also.set_initializer(shared);
            also.remove_initializer();
        }
        context
            .exclusive_module(&mut first)
            .run_passes("globaldce", None, &PassOptions::default())
            .expect("the pipeline");
        assert!(context.module(&first).get_global("dead").is_none());
        let again = context.constant_struct(&[one, one], false);
        assert_eq!(again.print_to_string(), "{ i32, i32 } { i32 1, i32 1 }");
        let global = context.module(&second).get_global("also").expect("the global");
        global.set_initializer(again);
        assert_eq!(context.module(&second).verify(), Ok(()));
    });
}

#[test]
fn decision_43_a_tracked_handle_keeps_a_place_across_a_pass_on_another_module() {
    with_context(|context| {
        let i32_type = context.get_type::<I32>();
        let first = context.create_module("first");
        let mut second = context.create_module("second");
        let handle = {
            let module = context.module(&first);
            module.track(module.add_global("kept", i32_type))
        };
        context
            .exclusive_module(&mut second)
            .run_passes("globaldce", None, &PassOptions::default())
            .expect("the pipeline");
        let back = handle.get(context.module(&first)).expect("still there");
        assert_eq!(back.name(), "kept");
    });
}

#[test]
fn decision_43_two_modules_can_be_built_side_by_side_and_then_linked() {
    with_context(|context| {
        let i32_type = context.get_type::<I32>();
        let mut first = context.create_module("first");
        let second = context.create_module("second");
        let in_first = context.module(&first).add_global("in_first", i32_type);
        let in_second = context.module(&second).add_global("in_second", i32_type);
        in_first.set_initializer(i32_type.constant_null());
        in_second.set_initializer(i32_type.constant_integer(1, false));
        context.exclusive_module(&mut first).link_in(second).expect("linked");
        let text = context.module(&first).print_to_string();
        assert!(text.contains("@in_first") && text.contains("@in_second"), "{text}");
        assert_eq!(context.module(&first).verify(), Ok(()));
    });
}

#[test]
fn decision_43_an_engine_shares_the_context_with_what_is_built_beside_it() {
    with_context(|context| {
        let module = context.parse_ir_text("define i32 @f() {\n ret i32 7\n}").expect("good IR");
        let engine = ExecutionEngine::interpreter(context, module).expect("an engine");
        let found = engine.find_function("f").expect("the function");
        let beside = context.create_module("beside");
        let global = context.module(&beside).add_global("g", context.get_type::<I32>());
        assert_eq!((found.name().as_str(), global.name().as_str()), ("f", "g"));
    });
}

struct MakesConstants;

impl ModulePass for MakesConstants {
    fn run(&mut self, module: &mut Module<'_>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
        let context = module.context();
        let one = context.get_type::<I32>().constant_integer(1, false);
        let pair = context.constant_struct(&[one, one], false);
        let global = module.add_global("made_in_the_pass", pair.get_type());
        global.set_initializer(pair);
        PreservedAnalyses::None
    }
}

#[test]
fn decision_43_a_pass_can_make_constants_through_its_module() {
    with_context(|context| {
        let mut module = context.create_module("m");
        context
            .exclusive_module(&mut module)
            .run_passes_with("makes-constants", None, &PassOptions::default(), |builder| {
                builder.register_module_pass("makes-constants", || MakesConstants);
            })
            .expect("the pipeline");
        assert!(context.module(&module).print_to_string().contains("{ i32, i32 } { i32 1, i32 1 }"));
    });
}

#[test]
fn decision_43_a_constant_from_the_context_can_be_used_by_a_builder_in_any_module() {
    with_context(|context| {
        let i32_type = context.get_type::<I32>();
        let one = i32_type.constant_integer(1, false);
        let pair = context.constant_struct(&[one, one], false);
        let program = context.create_module("m");
        let module = context.module(&program);
        let function = module.add_function("f", context.function_type(pair.get_type(), &[], false));
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        builder.return_value(pair);
        assert_eq!(module.verify(), Ok(()));
    });
}

// Nothing in the crate may hand out `'static` but plain data and inline assembly, since `'static` is what lets a value be held
// across a pass. A new one has to be added here, against D41's list, on purpose.
#[test]
fn decision_41_only_plain_data_carries_static() {
    const PLAIN: [&str; 17] = [
        "constant_integer",
        "constant_integer_from_words",
        "constant_integer_from_string",
        "constant_all_ones",
        "constant_float",
        "constant_float_from_string",
        "constant_float_from_bits",
        "constant_data_array",
        "constant_string",
        "constant_null",
        "undefined",
        "poison",
        "constant_byte",
        "constant_byte_from_words",
        "constant_byte_from_string",
        "inline_assembly",
        "metadata_string",
    ];
    let directory = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(directory).expect("the source") {
        let path = entry.expect("the source").path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("the source");
        for (start, _) in source.match_indices("pub fn ") {
            let rest = &source[start + "pub fn ".len()..];
            let name: String =
                rest.chars().take_while(|character| character.is_alphanumeric() || *character == '_').collect();
            let signature = &rest[..rest.find('{').unwrap_or(rest.len())];
            let returned = signature.rsplit("->").next().unwrap_or_default();
            if signature.contains("->")
                && (returned.contains("Value<'context, 'static") || returned.contains("Metadata<'context, 'static"))
            {
                found.push(name);
            }
        }
    }
    let unexpected: Vec<&String> = found.iter().filter(|name| !PLAIN.contains(&name.as_str())).collect();
    assert!(unexpected.is_empty(), "these hand out `'static` and aren't on D41's list: {unexpected:?}");
    assert!(found.len() >= PLAIN.len(), "the search found too little to mean anything: {found:?}");
}
