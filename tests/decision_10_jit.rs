//! Running generated code through the JIT.

use lyftar::jit::{ExecutionEngine, GenericValue, Jit, JitSymbolFlags};
use lyftar::{Bits, Builder, Integer, Value, with_context};

type I64 = Integer<Bits<64>>;

fn build_add_one<'context>(context: lyftar::Context<'context>) -> lyftar::Module<'context> {
    let module = context.create_module("jit");
    {
        let i64_type = context.get_type::<I64>();
        let function_type = context.function_type(i64_type.to_dynamic(), &[i64_type.to_dynamic()], false);
        let function = module.add_function("add_one", function_type);
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        let parameter = function.parameter(0).checked_cast::<I64>();
        let sum = builder.integer_add(parameter, i64_type.constant_integer(1, false), "sum");
        builder.return_value(sum);
    }
    module
}

#[test]
fn decision_10_lljit_runs_code_from_a_fresh_context() {
    let jit = Jit::new().expect("a JIT");
    jit.add_module(jit.main_dylib(), build_add_one).expect("adding the module");
    let address = jit.lookup("add_one").expect("the function");
    // SAFETY: this function has this signature, as built above.
    let add_one = unsafe { std::mem::transmute::<usize, extern "C" fn(i64) -> i64>(address as usize) };
    assert_eq!(add_one(41), 42);
}

extern "C" fn from_the_compiler(value: i64) -> i64 {
    value * 2
}

#[test]
fn decision_10_lljit_can_call_back_into_the_compiler_and_see_trackers_go() {
    let jit = Jit::new().expect("a JIT");
    let library = jit.main_dylib();
    let name = jit.mangle_and_intern("from_the_compiler");
    library
        .define_absolute_symbols(&[(
            &name,
            from_the_compiler as *const () as u64,
            JitSymbolFlags { exported: true, callable: true, ..JitSymbolFlags::default() },
        )])
        .expect("defining the symbol");
    let tracker = library.create_resource_tracker();
    jit.add_module_with_tracker(&tracker, |context| {
        let module = context.create_module("calls_back");
        {
            let i64_type = context.get_type::<I64>();
            let function_type = context.function_type(i64_type.to_dynamic(), &[i64_type.to_dynamic()], false);
            let callee = module.add_function("from_the_compiler", function_type);
            let function = module.add_function("twice", function_type);
            let builder = Builder::at_end_of(function.append_basic_block("entry"));
            let result = builder.call(callee, &[function.parameter(0)], "result");
            builder.return_value(result);
        }
        module
    })
    .expect("adding the module");
    let address = jit.lookup("twice").expect("the function");
    // SAFETY: this function has this signature, as built above.
    let twice = unsafe { std::mem::transmute::<usize, extern "C" fn(i64) -> i64>(address as usize) };
    assert_eq!(twice(21), 42);
    tracker.remove().expect("removing");
    assert!(jit.lookup("twice").is_err());
}

#[test]
fn decision_10_a_module_the_verifier_rejects_comes_back_as_an_error() {
    let jit = Jit::new().expect("a JIT");
    let result = jit.add_module(jit.main_dylib(), |context| {
        let module = context.create_module("broken");
        {
            let function_type = context.function_type(context.void_type(), &[], false);
            let function = module.add_function("broken", function_type);
            // A block with no terminator: the verifier's report comes back.
            let _ = function.append_basic_block("entry");
        }
        module
    });
    assert!(result.is_err_and(|error| error.message().contains("terminator")));
}

#[test]
fn decision_10_the_execution_engine_runs_code_inside_with_context() {
    with_context(|context| {
        let engine_module = build_add_one(context);
        let mut engine = ExecutionEngine::jit_compiler(engine_module, 2).expect("an engine");
        let address = engine.function_address("add_one");
        assert_ne!(address, 0);
        // SAFETY: this function has this signature, as built above.
        let add_one = unsafe { std::mem::transmute::<usize, extern "C" fn(i64) -> i64>(address as usize) };
        assert_eq!(add_one(1), 2);

        assert_eq!(engine.remove_module("jit").map(|module| module.identifier()), Ok("jit".to_string()));
    });
}

#[test]
fn decision_10_the_interpreter_runs_a_function_with_arguments() {
    with_context(|context| {
        let engine = ExecutionEngine::interpreter(build_add_one(context)).expect("an interpreter");
        let i64_type = context.get_type::<I64>();
        let found: Value<'_, '_, lyftar::Function> = engine.find_function("add_one").expect("the function");
        // SAFETY: as built above, and the one argument is an `i64`.
        let result = unsafe { engine.run_function(found, &[&GenericValue::from_integer(i64_type, 41, true)]) };
        assert_eq!(result.to_integer(true), 42);
    });
}
