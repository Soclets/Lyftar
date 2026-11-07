//! A build call makes no allocation of its own (D24). LLVM's own allocations don't pass through Rust's allocator, so anything
//! this counts is Lyftar's.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use lyftar::enums::IntegerPredicate;
use lyftar::{Bits, Builder, Function, Integer, Pointer, VariadicFunction, Void, with_context};

type I64 = Integer<Bits<64>>;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

struct Counting;

// SAFETY: this only counts, and passes everything on to the system allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        // SAFETY: the caller's promises are passed on.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller's promises are passed on.
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

#[test]
fn decision_24_build_calls_allocate_nothing() {
    with_context(|context| {
        let module = context.create_module("allocations");
        let module = context.module(&module);
        let i64_type = context.get_type::<I64>();
        let dynamic = i64_type.to_dynamic();
        let pointer_type = context.pointer_type(0).to_dynamic();
        let callee = module.add_function("callee", context.function_type(dynamic, &[dynamic, dynamic], false));
        let function =
            module.add_function("function", context.function_type(dynamic, &[dynamic, dynamic, pointer_type], false));
        let entry = function.append_basic_block("entry");
        let next = function.append_basic_block("next");
        let builder = Builder::at_end_of(entry);
        let left = function.parameter(0).checked_cast::<I64>();
        let right = function.parameter(1).checked_cast::<I64>();
        let pointer = function.parameter(2).checked_cast::<Pointer>();

        let specific = module.add_specific_function::<Function<I64, (I64, I64)>>("specific");
        let variadic = module.add_specific_function::<VariadicFunction<Void, (I64,)>>("variadic");
        let rest = [left.to_dynamic(); 3];

        let before = ALLOCATIONS.with(Cell::get);
        let specific_result = builder.call(specific, (left, right), "specific_result");
        builder.call(variadic, ((specific_result,), &rest), "");
        let sum = builder.integer_add(left, right, "sum");
        let compared = builder.integer_compare(IntegerPredicate::Equal, sum, right, "compared");
        let loaded = builder.load(i64_type, pointer, "loaded");
        builder.store(loaded, pointer);
        let element = builder.get_element_pointer(dynamic, pointer, &[left], "element");
        let called = builder.call(callee, &[left, right], "called");
        let chosen = builder.select(compared, sum, loaded, "chosen");
        let phi_input = builder.integer_multiply(chosen, called.to_dynamic().checked_cast::<I64>(), "phi_input");
        builder.store(phi_input, element);
        builder.branch(next);
        builder.position_at_end(next);
        builder.return_value(sum);
        let after = ALLOCATIONS.with(Cell::get);

        assert_eq!(after - before, 0, "{} allocations in Lyftar's build calls", after - before);
        assert_eq!(module.verify(), Ok(()));
    });
}

/// A stack map with one statepoint's record, returning to 0x1010, with a deoptimization value and two pairs of roots.
fn stack_map() -> Vec<u8> {
    let mut bytes = vec![3, 0, 0, 0];
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(0x1000u64.to_le_bytes());
    bytes.extend(32u64.to_le_bytes());
    bytes.extend(1u64.to_le_bytes());
    bytes.extend(99u64.to_le_bytes());
    bytes.extend(1u64.to_le_bytes());
    bytes.extend(0x10u32.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(8u16.to_le_bytes());
    for (kind, value) in [(4u8, 0i32), (4, 0), (4, 1), (5, 0), (3, 8), (3, 16), (3, 24), (3, 32)] {
        bytes.push(kind);
        bytes.push(0);
        bytes.extend(8u16.to_le_bytes());
        bytes.extend(31u16.to_le_bytes());
        bytes.extend(0u16.to_le_bytes());
        bytes.extend(value.to_le_bytes());
    }
    while bytes.len() % 8 != 0 {
        bytes.push(0);
    }
    bytes.extend(0u32.to_le_bytes());
    while bytes.len() % 8 != 0 {
        bytes.push(0);
    }
    bytes
}

// trace: D24, D45
#[test]
fn decision_24_a_stack_map_lookup_allocates_nothing() {
    let section = stack_map();
    let mut stack_maps = lyftar_stack_maps::StackMaps::new();
    stack_maps.add_section(&section).expect("a good stack map");
    let before = ALLOCATIONS.with(Cell::get);
    let call = stack_maps.find(0x1010).expect("the call");
    let statepoint = call.record.statepoint().expect("a statepoint");
    let roots = statepoint.roots().count();
    let deoptimization = statepoint.deoptimization().count();
    let locations = call.record.locations().count();
    let missing = stack_maps.find(0x1011).is_none();
    let after = ALLOCATIONS.with(Cell::get);
    assert_eq!(after - before, 0, "{} allocations in a lookup", after - before);
    assert_eq!((roots, deoptimization, locations, missing), (2, 1, 8, true));
}
