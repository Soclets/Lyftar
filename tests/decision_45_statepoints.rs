//! Statepoints written directly, and what's refused at the call (D3, D45).

use lyftar::garbage_collection::{SafepointPlacement, StatepointCall, StatepointFlags, statepoint_pipeline};
use lyftar::passes::PassOptions;
use lyftar::target::{
    CodeGenerationFileType, CodeGenerationOptimizationLevel, CodeModel, RelocationMode, Target, TargetMachine,
};
use lyftar::{
    AddressSpace, Bits, Builder, Context, Function, Integer, Module, OwnedModule, Pointer, Value, Void, with_context,
};

type ManagedPointer = Pointer<AddressSpace<1>>;
type I64 = Integer<Bits<64>>;
type I8 = Integer<Bits<8>>;

const GARBAGE_COLLECTOR: &str = "statepoint-example";

/// A function using `statepoint-example`, with a managed pointer as its parameter, and a builder in it.
fn scene<'context, 'module>(
    module: &'module Module<'context>,
) -> (Builder<'context, 'module>, Value<'context, 'module, Function<Void, (ManagedPointer,)>>) {
    let function = module.add_specific_function::<Function<Void, (ManagedPointer,)>>("program");
    function.set_garbage_collector(GARBAGE_COLLECTOR);
    (Builder::at_end_of(function.append_basic_block("entry")), function)
}

fn call<'call, 'context, 'module>(
    callee: Value<'context, 'module, Function<Void, ()>>,
    live: &'call [Value<'context, 'module>],
) -> StatepointCall<'call, 'context, 'module> {
    StatepointCall {
        id: 7,
        patch_size_in_bytes: 0,
        function_type: callee.value_type(),
        callee: callee.to_dynamic(),
        flags: StatepointFlags::NONE,
        arguments: &[],
        transition: &[],
        deoptimization: &[],
        live,
    }
}

fn emits<'context>(context: &mut Context<'context>, owned: &mut OwnedModule<'context>) -> Vec<u8> {
    let machine = TargetMachine::for_host().expect("a machine");
    context.module(owned).set_target_machine(&machine);
    let object = machine
        .emit_to_memory_buffer(context.exclusive_module(owned), CodeGenerationFileType::Object)
        .expect("code generation");
    object.as_bytes().to_vec()
}

#[test]
fn decision_45_a_statepoint_written_directly_has_its_bundles_relocates_and_result() {
    with_context(|context| {
        let mut owned = context.create_module("direct");
        let module = context.module(&owned);
        let (builder, function) = scene(module);
        let callee = module.add_specific_function::<Function<I64, (I64,)>>("callee");
        let i64_type = context.get_type::<I64>();
        let object = function.parameter(0);
        let derived = builder
            .get_element_pointer(
                context.get_type::<I8>().to_dynamic(),
                function.parameter(0).checked_cast::<ManagedPointer>(),
                &[i64_type.constant_integer(16, false)],
                "derived",
            )
            .to_dynamic();
        let argument = i64_type.constant_integer(5, false).to_dynamic();
        let frame_state = i64_type.constant_integer(11, false).to_dynamic();
        let transition = i64_type.constant_integer(13, false).to_dynamic();
        let statepoint = builder.garbage_collection_statepoint(
            &StatepointCall {
                id: 99,
                patch_size_in_bytes: 0,
                function_type: callee.value_type(),
                callee: callee.to_dynamic(),
                flags: StatepointFlags::TRANSITION | StatepointFlags::DEOPTIMIZATION_LIVE_IN,
                arguments: &[argument],
                transition: &[transition],
                deoptimization: &[frame_state],
                live: &[object, derived],
            },
            "statepoint",
        );
        assert_eq!(statepoint.id(), 99);
        assert_eq!(statepoint.patch_size_in_bytes(), 0);
        assert_eq!(statepoint.flag_bits(), 3);
        assert_eq!(statepoint.call_arguments().len(), 1);
        assert_eq!(statepoint.live().len(), 2);
        assert_eq!(statepoint.deoptimization().len(), 1);
        assert_eq!(statepoint.transition().len(), 1);
        assert_eq!(statepoint.callee().name(), "callee");
        let result = builder.garbage_collection_result(statepoint, "result").checked_cast::<I64>();
        let relocated = builder.garbage_collection_relocate(statepoint, 0, 1, "relocated");
        let relocated = relocated.checked_cast::<ManagedPointer>();
        builder.store(result, relocated);
        builder.return_void();
        drop(builder);
        let printed = module.print_to_string();
        assert_eq!(module.verify(), Ok(()), "{printed}");
        for piece in ["\"deopt\"(i64 11)", "\"gc-transition\"(i64 13)", "\"gc-live\"(", "elementtype(i64 (i64))"] {
            assert!(printed.contains(piece), "{piece} is missing from {printed}");
        }
        // Only x86 can make code for the transition flag, and on anything else LLVM would end the process.
        let machine_for = |triple: &str| {
            Target::from_triple(triple).expect("LLVM's target").create_target_machine(
                triple,
                "",
                "",
                CodeGenerationOptimizationLevel::Default,
                RelocationMode::Default,
                CodeModel::Default,
            )
        };
        let x86 = machine_for("x86_64-unknown-linux-gnu");
        module.set_target_machine(&x86);
        let object = x86
            .emit_to_memory_buffer(context.exclusive_module(&mut owned), CodeGenerationFileType::Object)
            .expect("x86 code");
        assert!(!object.as_bytes().is_empty());
        let aarch64 = machine_for("aarch64-unknown-linux-gnu");
        let error = aarch64
            .emit_to_memory_buffer(context.exclusive_module(&mut owned), CodeGenerationFileType::Object)
            .err()
            .expect("a transition AArch64 can't make");
        assert!(error.message().contains("only x86 can make code"), "{}", error.message());
    });
}

#[test]
fn decision_45_a_statepoint_written_directly_emits_with_deoptimization_values() {
    with_context(|context| {
        let mut owned = context.create_module("deoptimization");
        let module = context.module(&owned);
        let (builder, function) = scene(module);
        let callee = module.add_specific_function::<Function<Void, ()>>("callee");
        let frame_state = context.get_type::<I64>().constant_integer(11, false).to_dynamic();
        let object = function.parameter(0);
        builder.garbage_collection_statepoint(
            &StatepointCall {
                flags: StatepointFlags::DEOPTIMIZATION_LIVE_IN,
                deoptimization: &[frame_state],
                ..call(callee, &[object])
            },
            "",
        );
        builder.return_void();
        drop(builder);
        assert!(!emits(context, &mut owned).is_empty());
    });
}

#[test]
fn decision_45_an_invoke_statepoint_relocates_on_both_paths() {
    with_context(|context| {
        let mut owned = context.create_module("invoke");
        let module = context.module(&owned);
        let (builder, function) = scene(module);
        let callee = module.add_specific_function::<Function<Void, ()>>("callee");
        let personality = module.add_specific_function::<Function<Integer<Bits<32>>, ()>>("__gxx_personality_v0");
        function.set_personality(personality);
        let normal = function.append_basic_block("normal");
        let unwind = function.append_basic_block("unwind");
        let object = function.parameter(0);
        let statepoint =
            builder.garbage_collection_statepoint_invoke(&call(callee, &[object]), normal, unwind, "statepoint");
        builder.position_at_end(normal);
        builder.garbage_collection_relocate(statepoint, 0, 0, "relocated");
        builder.return_void();
        builder.position_at_end(unwind);
        let landing_pad = builder.landing_pad(context.token_type(), None, 0, "landing_pad");
        landing_pad.set_cleanup(true);
        builder.garbage_collection_relocate_on_unwind(landing_pad, 0, 0, "relocated_on_unwind");
        builder.unreachable();
        drop(builder);
        assert_eq!(module.verify(), Ok(()), "{}", module.print_to_string());
        assert!(!emits(context, &mut owned).is_empty());
    });
}

#[test]
fn decision_45_place_safepoints_polls_then_the_rewrite_relocates() {
    with_context(|context| {
        let machine = TargetMachine::for_host().expect("a machine");
        let mut owned = context.create_module("polls");
        let module = context.module(&owned);
        module.set_target_machine(&machine);
        let slow_path = module.add_specific_function::<Function<Void, ()>>("poll_slow_path");
        let poll = module.add_specific_function::<Function<Void, ()>>("gc.safepoint_poll");
        let poll_builder = Builder::at_end_of(poll.append_basic_block("entry"));
        poll_builder.call(slow_path, (), "");
        poll_builder.return_void();
        drop(poll_builder);
        let consume = module.add_specific_function::<Function<Void, (ManagedPointer,)>>("consume");
        consume.add_target_dependent_attribute("gc-leaf-function", "");
        let (builder, function) = scene(module);
        builder.call(consume, (function.parameter(0).checked_cast::<ManagedPointer>(),), "");
        builder.call(consume, (function.parameter(0).checked_cast::<ManagedPointer>(),), "");
        builder.return_void();
        drop(builder);
        context
            .exclusive_module(&mut owned)
            .run_passes(
                &statepoint_pipeline("", SafepointPlacement::PlacePolls),
                Some(&machine),
                &PassOptions::default(),
            )
            .expect("the pipeline");
        let printed = context.module(&owned).print_to_string();
        assert!(printed.contains("gc.statepoint"), "{printed}");
        assert!(printed.contains("poll_slow_path"), "{printed}");
        assert_eq!(context.module(&owned).verify(), Ok(()));
    });
}

#[test]
fn decision_45_place_safepoints_without_a_poll_is_an_error() {
    with_context(|context| {
        let mut owned = context.create_module("no poll");
        let (builder, _) = scene(context.module(&owned));
        builder.return_void();
        drop(builder);
        let error = context
            .exclusive_module(&mut owned)
            .run_passes("function(place-safepoints)", None, &PassOptions::default())
            .expect_err("no poll");
        assert!(error.message().contains("gc.safepoint_poll"), "{}", error.message());
    });
}

#[test]
fn decision_45_the_pipeline_runs_optimizations_then_polls_then_the_rewrite() {
    assert_eq!(statepoint_pipeline("", SafepointPlacement::CallsOnly), "rewrite-statepoints-for-gc");
    assert_eq!(
        statepoint_pipeline("default<O2>", SafepointPlacement::PlacePolls),
        "default<O2>,function(place-safepoints),rewrite-statepoints-for-gc"
    );
}

#[test]
fn decision_45_text_with_an_unknown_strategy_or_a_relocate_without_one_is_an_error() {
    with_context(|context| {
        let unknown = context.parse_ir_text("define void @f() gc \"nothing-registered\" { ret void }\n");
        assert!(unknown.err().expect("no such strategy").message().contains("nothing-registered"));
        let no_collector = context.parse_ir_text(
            "declare ptr addrspace(1) @llvm.experimental.gc.relocate.p1(token, i32, i32)\n\
             define void @f() { %r = call ptr addrspace(1) @llvm.experimental.gc.relocate.p1(token undef, i32 0, i32 0) ret void }\n",
        );
        assert!(no_collector.err().expect("no collector").message().contains("no garbage collector"));
    });
}

macro_rules! refused {
    ($name:ident, $expected:literal, |$context:ident, $module:ident, $builder:ident, $function:ident, $callee:ident| $body:block) => {
        #[test]
        #[should_panic(expected = $expected)]
        fn $name() {
            with_context(|$context| {
                let owned = $context.create_module("refused");
                let $module = $context.module(&owned);
                let ($builder, $function) = scene($module);
                let $callee = $module.add_specific_function::<Function<Void, ()>>("callee");
                $body
            });
        }
    };
}

refused!(
    decision_3_a_statepoint_in_a_function_with_no_collector_panics,
    "garbage_collection_statepoint: this function has no garbage collector",
    |_context, module, _builder, _function, callee| {
        let plain = module.add_specific_function::<Function<Void, ()>>("plain");
        let builder = Builder::at_end_of(plain.append_basic_block("entry"));
        builder.garbage_collection_statepoint(&call(callee, &[]), "");
    }
);
refused!(
    decision_3_too_much_patch_space_panics,
    "bytes of patch space as a positive 32-bit number",
    |_context, _module, builder, _function, callee| {
        builder.garbage_collection_statepoint(
            &StatepointCall { patch_size_in_bytes: 0x8000_0000, ..call(callee, &[]) },
            "",
        );
    }
);
refused!(
    decision_3_a_statepoint_of_a_variadic_function_returning_something_panics,
    "garbage_collection_statepoint: LLVM can only wrap a variadic function that returns void",
    |context, module, builder, _function, _callee| {
        let i64_dynamic = context.get_type::<I64>().to_dynamic();
        let function_type = context.function_type(i64_dynamic, &[], true);
        let variadic = module.add_function("variadic", function_type);
        builder.garbage_collection_statepoint(
            &StatepointCall {
                id: 0,
                patch_size_in_bytes: 0,
                function_type,
                callee: variadic.to_dynamic(),
                flags: StatepointFlags::NONE,
                arguments: &[],
                transition: &[],
                deoptimization: &[],
                live: &[],
            },
            "",
        );
    }
);
refused!(
    decision_3_an_integer_in_the_live_list_panics,
    "isn't a pointer, and only a pointer reaches the stack map",
    |context, _module, builder, _function, callee| {
        let integer = context.get_type::<I64>().constant_integer(1, false).to_dynamic();
        builder.garbage_collection_statepoint(&call(callee, &[integer]), "");
    }
);
refused!(
    decision_3_a_pointer_the_strategy_does_not_manage_in_the_live_list_panics,
    "is a pointer this function's garbage collector doesn't manage",
    |context, _module, builder, _function, callee| {
        let unmanaged = context.get_type::<Pointer>().constant_null().to_dynamic();
        builder.garbage_collection_statepoint(&call(callee, &[unmanaged]), "");
    }
);
refused!(
    decision_3_the_result_of_a_call_that_returns_nothing_panics,
    "the call this statepoint wraps returns nothing",
    |_context, _module, builder, _function, callee| {
        let statepoint = builder.garbage_collection_statepoint(&call(callee, &[]), "");
        builder.garbage_collection_result(statepoint, "");
    }
);
refused!(
    decision_3_a_relocate_index_out_of_bounds_panics,
    "is past this statepoint's 1 live values",
    |_context, _module, builder, function, callee| {
        let object = function.parameter(0);
        let statepoint = builder.garbage_collection_statepoint(&call(callee, &[object]), "");
        builder.garbage_collection_relocate(statepoint, 0, 1, "");
    }
);
refused!(
    decision_3_relocating_a_stack_allocation_panics,
    "is not a pointer this function's garbage collector manages",
    |context, _module, builder, _function, callee| {
        let slot = builder.stack_allocate(context.get_type::<ManagedPointer>(), "slot").to_dynamic();
        let statepoint = builder.garbage_collection_statepoint(&call(callee, &[slot]), "");
        builder.garbage_collection_relocate(statepoint, 0, 0, "");
    }
);
refused!(
    decision_3_a_relocate_on_unwind_with_more_than_one_predecessor_panics,
    "garbage_collection_relocate_on_unwind: a statepoint's landing pad has to have exactly one predecessor",
    |context, _module, builder, function, _callee| {
        let landing_type = context.token_type();
        let first = function.append_basic_block("first");
        let second = function.append_basic_block("second");
        let pad = function.append_basic_block("pad");
        builder.conditional_branch(context.get_type::<Integer<Bits<1>>>().constant_integer(0, false), first, second);
        builder.position_at_end(first);
        builder.branch(pad);
        builder.position_at_end(second);
        builder.branch(pad);
        builder.position_at_end(pad);
        let landing_pad = builder.landing_pad(landing_type, None, 0, "");
        builder.garbage_collection_relocate_on_unwind(landing_pad, 0, 0, "");
    }
);
refused!(
    decision_3_a_relocate_on_unwind_after_something_else_panics,
    "garbage_collection_relocate_on_unwind: this landing pad's predecessor doesn't end in a statepoint",
    |context, _module, builder, function, _callee| {
        let landing_type = context.token_type();
        let pad = function.append_basic_block("pad");
        builder.branch(pad);
        builder.position_at_end(pad);
        let landing_pad = builder.landing_pad(landing_type, None, 0, "");
        builder.garbage_collection_relocate_on_unwind(landing_pad, 0, 0, "");
    }
);
refused!(
    decision_3_a_relocate_on_a_landing_pad_that_is_not_a_token_panics,
    "garbage_collection_relocate_on_unwind: a gc.relocate reads a landing pad of type `token`",
    |context, _module, builder, function, _callee| {
        let pad = function.append_basic_block("pad");
        builder.branch(pad);
        builder.position_at_end(pad);
        let landing_pad = builder.landing_pad(context.get_type::<Pointer>(), None, 0, "");
        builder.garbage_collection_relocate_on_unwind(landing_pad, 0, 0, "");
    }
);
refused!(
    decision_3_a_root_outside_the_entry_block_panics,
    "isn't a fixed-size stack allocation in the entry block",
    |context, _module, builder, function, _callee| {
        let next = function.append_basic_block("next");
        builder.branch(next);
        builder.position_at_end(next);
        let pointer = context.get_type::<Pointer>();
        let slot = builder.stack_allocate(pointer, "slot");
        builder.garbage_collection_root(slot, pointer.constant_null());
    }
);
refused!(
    decision_3_a_root_with_metadata_that_is_not_constant_panics,
    "isn't a constant",
    |context, _module, builder, _function, _callee| {
        let pointer = context.get_type::<Pointer>();
        let slot = builder.stack_allocate(pointer, "slot");
        let not_constant = builder.stack_allocate(pointer, "other");
        builder.garbage_collection_root(slot, not_constant);
    }
);
refused!(
    decision_3_a_root_of_something_else_with_no_metadata_panics,
    "holds something other than a pointer, and that needs metadata",
    |context, _module, builder, _function, _callee| {
        let slot = builder.stack_allocate(context.get_type::<I64>(), "slot");
        builder.garbage_collection_root(slot, context.get_type::<Pointer>().constant_null());
    }
);
