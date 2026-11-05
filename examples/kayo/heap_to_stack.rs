//! kayo's heap-to-stack pass (CyborgPL/kayo/llvm-passes/heap_to_stack.cpp), in Rust.
//!
//! Memory the runtime hands out with `cyborg_alloc`, of a size known here and small, that never leaves the function that
//! asked for it, is that function's own stack memory instead, and its `cyborg_free` goes. A text's free whose length says it
//! was never allocated (the `STATIC` bit, a literal) does nothing, so the call goes.
//!
//! "Never leaves" is read from every use of the pointer, through the addresses made from it: loads from it and stores into
//! it, copies and fills of it, the runtime's `cyborg_out_*` (which read bytes out and keep nothing), the program's own
//! functions it is lent to that LLVM found keep nothing of it, and the one `cyborg_free` of it. Anything else and it stays on
//! the heap.

use lyftar::module::ValueToErase;
use lyftar::passes::{Analyses, FunctionInPass, FunctionPass, PreservedAnalyses};
use lyftar::values::{
    CallBase, ConstantInteger, GetElementPointerInstruction, IntrinsicInstruction, LoadInstruction, MemoryIntrinsic,
    StackAllocationInstruction, StoreInstruction,
};
use lyftar::{Builder, Integer, Value};

/// What one function's frame takes in all, at most: past it the rest stay on the heap.
const MOST_IN_BYTES: u64 = 4096;

/// The runtime's `STATIC` bit in a text's length.
const STATIC: u64 = 1 << 63;

/// Heap to stack, as a function pass.
pub struct HeapToStack;

fn calls<'context>(call: Value<'context, '_, CallBase>, name: &str) -> bool {
    call.called_function().is_some_and(|callee| callee.name() == name)
}

/// Whether `call` hands `value` only to parameters of a function defined here that LLVM has found keep nothing of it. Freeing
/// it would be keeping it, so such a callee never frees it either.
fn lent_only<'context, 'module>(call: Value<'context, 'module, CallBase>, value: Value<'context, 'module>) -> bool {
    let Some(callee) = call.called_function() else {
        return false;
    };
    if callee.is_declaration() {
        return false;
    }
    let mut handed = false;
    for index in 0..call.argument_count() {
        if call.argument(index) != value {
            continue;
        }
        if !call.does_not_capture(index) {
            return false;
        }
        handed = true;
    }
    handed
}

/// Whether every use of `pointer`, and of every address made from it, keeps it in this function. The frees of it go in
/// `frees`.
fn stays<'context, 'module>(
    pointer: Value<'context, 'module>,
    frees: &mut Vec<Value<'context, 'module, CallBase>>,
) -> bool {
    let mut work = vec![pointer];
    let mut seen: Vec<Value<'context, 'module>> = Vec::new();
    while let Some(value) = work.pop() {
        if seen.contains(&value) {
            continue;
        }
        seen.push(value);
        for found in value.uses() {
            let user = found.user();
            if let Some(load) = user.as_kind::<LoadInstruction>() {
                if load.pointer_operand() != value || load.is_volatile() {
                    return false;
                }
            } else if let Some(store) = user.as_kind::<StoreInstruction>() {
                if store.pointer_operand() != value || store.value_operand() == value || store.is_volatile() {
                    return false;
                }
            } else if let Some(address) = user.as_kind::<GetElementPointerInstruction>() {
                if address.pointer_operand() != value {
                    return false;
                }
                work.push(address.to_dynamic());
            } else if let Some(memory) = user.as_kind::<MemoryIntrinsic>() {
                // A copy's or a fill's address, never its length.
                if memory.is_volatile_memory() || memory.length_in_bytes() == value {
                    return false;
                }
            } else if let Some(call) = user.as_kind::<CallBase>() {
                let is_lifetime = call
                    .as_kind::<IntrinsicInstruction>()
                    .is_some_and(lyftar::Value::<'_, '_, IntrinsicInstruction>::is_lifetime_start_or_end);
                if calls(call, "cyborg_free") && value == pointer && call.argument(0) == value {
                    frees.push(call);
                } else if call.called_function().is_some_and(|callee| callee.name().starts_with("cyborg_out_")) {
                    // Bytes read out of it into a print; the pointer is not kept.
                } else if is_lifetime {
                } else if !lent_only(call, value) {
                    return false;
                }
            } else {
                return false;
            }
        }
    }
    true
}

impl FunctionPass for HeapToStack {
    fn run<'context>(
        &mut self,
        function: &mut FunctionInPass<'context>,
        _analyses: &Analyses<'_>,
    ) -> PreservedAnalyses {
        if function.function().is_declaration() {
            return PreservedAnalyses::All;
        }
        let mut allocations = Vec::new();
        let mut static_frees: Vec<ValueToErase> = Vec::new();
        for instruction in function.function().function_instructions() {
            let Some(call) = instruction.as_kind::<CallBase>() else {
                continue;
            };
            if calls(call, "cyborg_alloc") {
                let size = call.argument(0).as_kind::<ConstantInteger>().and_then(|found| found.zero_extended_value());
                // Zero bytes is the runtime's null pointer, not memory.
                if size.is_some_and(|size_in_bytes| size_in_bytes > 0 && size_in_bytes <= MOST_IN_BYTES) {
                    allocations.push(function.track(call));
                }
            } else if calls(call, "cyborg_str_free") && call.argument_count() == 2 {
                let length = call.argument(1).as_kind::<ConstantInteger>();
                if length.is_some_and(|found| {
                    found.width_in_bits() == 64 && found.zero_extended_value().is_some_and(|bits| bits & STATIC != 0)
                }) {
                    static_frees.push(function.value_to_erase(call));
                }
            }
        }
        let mut changed = !static_frees.is_empty();
        function.erase(&static_frees);

        // What earlier runs of this pass already moved here.
        let mut taken_in_bytes = 0;
        if let Some(entry) = function.function().entry_block() {
            for instruction in entry.instructions() {
                if let Some(slot) = instruction.as_kind::<StackAllocationInstruction>()
                    && slot.name().starts_with("on_stack")
                {
                    taken_in_bytes += slot.allocation_size_in_bytes().unwrap_or(0);
                }
            }
        }

        for allocation in &allocations {
            let to_erase = {
                let Some(call) = allocation.get(function.module()) else {
                    continue;
                };
                let Some(size_in_bytes) =
                    call.argument(0).as_kind::<ConstantInteger>().and_then(|found| found.zero_extended_value())
                else {
                    continue;
                };
                if taken_in_bytes + size_in_bytes > MOST_IN_BYTES {
                    continue;
                }
                let mut frees = Vec::new();
                if !stays(call.to_dynamic(), &mut frees) {
                    continue;
                }
                taken_in_bytes += size_in_bytes;
                let Some(entry) = function.function().entry_block() else {
                    continue;
                };
                let context = function.module().context();
                let builder = Builder::at_first_insertion_point(entry);
                let bytes = context.get_type::<Integer<lyftar::Bits<8>>>().array_type(size_in_bytes);
                let slot = builder.stack_allocate(bytes, "on_stack");
                // As `malloc` aligns: 16 bytes.
                slot.set_alignment_in_bytes(16);
                let mut to_erase: Vec<ValueToErase> = frees.iter().map(|free| function.value_to_erase(*free)).collect();
                call.replace_all_uses_with(slot);
                to_erase.push(function.value_to_erase(call));
                to_erase
            };
            function.erase(&to_erase);
            changed = true;
        }
        if changed { PreservedAnalyses::None } else { PreservedAnalyses::All }
    }
}
