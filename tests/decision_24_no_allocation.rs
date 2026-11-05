//! A build call makes no allocation of its own (D24). LLVM's own allocations don't pass through Rust's allocator, so anything
//! this counts is Lyftar's.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use lyftar::enums::IntegerPredicate;
use lyftar::{Bits, Builder, Integer, Pointer, with_context};

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

        let before = ALLOCATIONS.with(Cell::get);
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
