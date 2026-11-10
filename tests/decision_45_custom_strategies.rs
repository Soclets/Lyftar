//! A garbage collection strategy written in Rust (D45), linked in and as a plugin. `decision_45_moving_collection.rs` runs one
//! through the moving collector.

use lyftar::Type;
use lyftar::garbage_collection::{GarbageCollectionStrategy, register_strategy, strategy_is_registered};
use lyftar::passes::PassOptions;
use lyftar::{AddressSpace, Builder, Function, Pointer, Void, with_context};

struct OnlyAddressSpaceSeven;

impl GarbageCollectionStrategy for OnlyAddressSpaceSeven {
    const NAME: &'static str = "only-address-space-seven";
    const DESCRIPTION: &'static str = "manages the pointers in address space 7";

    fn manages_pointer(&self, pointer_type: Type<'_>) -> Option<bool> {
        Some(pointer_type.address_space() == 7)
    }
}

struct AnotherWithTheSameName;

impl GarbageCollectionStrategy for AnotherWithTheSameName {
    const NAME: &'static str = "only-address-space-seven";
    const DESCRIPTION: &'static str = "not the same one";

    fn manages_pointer(&self, _pointer_type: Type<'_>) -> Option<bool> {
        None
    }
}

struct NotRewritten;

impl GarbageCollectionStrategy for NotRewritten {
    const NAME: &'static str = "not-rewritten";
    const DESCRIPTION: &'static str = "writes its own statepoints";
    const REWRITES_STATEPOINTS: bool = false;

    fn manages_pointer(&self, _pointer_type: Type<'_>) -> Option<bool> {
        None
    }
}

struct NameWithNul;

impl GarbageCollectionStrategy for NameWithNul {
    const NAME: &'static str = "with\0nul";
    const DESCRIPTION: &'static str = "";

    fn manages_pointer(&self, _pointer_type: Type<'_>) -> Option<bool> {
        None
    }
}

#[test]
fn decision_45_a_strategy_in_rust_decides_which_pointers_are_rewritten() {
    assert!(!strategy_is_registered("never-registered"));
    register_strategy(OnlyAddressSpaceSeven);
    register_strategy(OnlyAddressSpaceSeven);
    assert!(strategy_is_registered(OnlyAddressSpaceSeven::NAME));
    with_context(|context| {
        let mut owned = context.create_module("custom");
        let module = context.module(&owned);
        let safepoint = module.add_specific_function::<Function<Void, ()>>("safepoint");
        let consume_managed =
            module.add_specific_function::<Function<Void, (Pointer<AddressSpace<7>>,)>>("consume_managed");
        let consume_other =
            module.add_specific_function::<Function<Void, (Pointer<AddressSpace<1>>,)>>("consume_other");
        let program = module
            .add_specific_function::<Function<Void, (Pointer<AddressSpace<7>>, Pointer<AddressSpace<1>>)>>("program");
        program.set_garbage_collector(OnlyAddressSpaceSeven::NAME);
        assert_eq!(program.garbage_collector().as_deref(), Some(OnlyAddressSpaceSeven::NAME));
        let builder = Builder::at_end_of(program.append_basic_block("entry"));
        builder.call(safepoint, (), "");
        builder.call(consume_managed, (program.parameter(0).checked_cast(),), "");
        builder.call(consume_other, (program.parameter(1).checked_cast(),), "");
        builder.return_void();
        drop(builder);
        context
            .exclusive_module(&mut owned)
            .run_passes("rewrite-statepoints-for-gc", None, &PassOptions::default())
            .expect("the pass");
        let printed = context.module(&owned).print_to_string();
        assert!(printed.contains("\"gc-live\"(ptr addrspace(7)"), "{printed}");
        assert!(!printed.contains("\"gc-live\"(ptr addrspace(1)"), "{printed}");
        assert!(printed.contains("llvm.experimental.gc.relocate.p7"), "{printed}");
    });
}

#[test]
fn decision_45_a_strategy_can_leave_statepoints_to_the_program() {
    register_strategy(NotRewritten);
    with_context(|context| {
        let mut owned = context.create_module("not rewritten");
        let module = context.module(&owned);
        let safepoint = module.add_specific_function::<Function<Void, ()>>("safepoint");
        let program = module.add_specific_function::<Function<Void, ()>>("program");
        program.set_garbage_collector(NotRewritten::NAME);
        let builder = Builder::at_end_of(program.append_basic_block("entry"));
        builder.call(safepoint, (), "");
        builder.return_void();
        drop(builder);
        context
            .exclusive_module(&mut owned)
            .run_passes("rewrite-statepoints-for-gc", None, &PassOptions::default())
            .expect("the pass");
        assert!(!context.module(&owned).print_to_string().contains("gc.statepoint"));
    });
}

#[test]
#[should_panic(expected = "register_strategy: another strategy is called \"statepoint-example\"")]
fn decision_3_a_strategy_named_as_a_built_in_panics() {
    struct Built;
    impl GarbageCollectionStrategy for Built {
        const NAME: &'static str = "statepoint-example";
        const DESCRIPTION: &'static str = "";
        fn manages_pointer(&self, _pointer_type: Type<'_>) -> Option<bool> {
            None
        }
    }
    register_strategy(Built);
}

#[test]
#[should_panic(expected = "register_strategy: another strategy is called \"only-address-space-seven\"")]
fn decision_3_two_strategies_with_one_name_panics() {
    register_strategy(OnlyAddressSpaceSeven);
    register_strategy(AnotherWithTheSameName);
}

#[test]
#[should_panic(expected = "has a NUL byte inside it")]
fn decision_3_a_strategy_name_with_a_nul_panics() {
    register_strategy(NameWithNul);
}

// Plugins are a compile error on Windows (D44).
#[cfg(not(target_os = "windows"))]
#[test]
fn decision_45_a_strategy_in_rust_works_as_a_plugin_where_plugins_work() {
    use std::process::Command;
    let built = Command::new(env!("CARGO"))
        .args(["build", "--example", "strategy_plugin"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("running cargo");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let mut path = std::env::current_exe().expect("this test's path");
    path.pop();
    path.pop();
    let plugin =
        path.join(format!("examples/{}strategy_plugin{}", std::env::consts::DLL_PREFIX, std::env::consts::DLL_SUFFIX));
    let directory = std::env::temp_dir().join(format!("lyftar-strategy-plugin-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a temporary directory");
    let input = directory.join("in.ll");
    std::fs::write(
        &input,
        "declare void @safepoint()\n\
         declare void @consume(ptr addrspace(5))\n\
         define void @program(ptr addrspace(5) %object, ptr addrspace(1) %other) gc \"address-space-five\" {\n\
         \x20 call void @safepoint()\n\
         \x20 call void @consume(ptr addrspace(5) %object)\n\
         \x20 ret void\n\
         }\n",
    )
    .expect("writing the IR");
    let llvm_config = env!("LYFTAR_LLVM_CONFIG_IN_USE");
    let binaries = Command::new(llvm_config).arg("--bindir").output().expect("llvm-config");
    let opt = std::path::Path::new(String::from_utf8_lossy(&binaries.stdout).trim()).join("opt");
    let output = Command::new(opt)
        .arg(format!("-load-pass-plugin={}", plugin.display()))
        .args(["-passes=rewrite-statepoints-for-gc", "-S"])
        .arg(&input)
        .output()
        .expect("running opt");
    let _ = std::fs::remove_dir_all(directory);
    assert!(output.status.success(), "opt failed: {}", String::from_utf8_lossy(&output.stderr));
    let printed = String::from_utf8_lossy(&output.stdout);
    assert!(printed.contains("gc.statepoint"), "{printed}");
    assert!(printed.contains("\"gc-live\"(ptr addrspace(5)"), "{printed}");
}
