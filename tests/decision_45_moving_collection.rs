//! The test D45 is for: a program compiled with Lyftar allocates through a small moving collector, with collections forced while
//! objects are held across calls, and what it prints proves the objects moved and every root was rewritten. It runs linked, with
//! no LLVM in the program, and through both JITs, with optimization on and off, for each strategy that can move objects.

#![cfg(unix)]

#[path = "collector/src/collector.rs"]
#[allow(dead_code)]
mod collector;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use lyftar::attributes::{AttributeKind, AttributeLocation};
use lyftar::enums::{IntegerPredicate, Linkage};
use lyftar::garbage_collection::{
    GarbageCollectionStrategy, SafepointPlacement, StatepointCall, StatepointFlags, register_strategy,
    statepoint_pipeline,
};
use lyftar::jit::{ExecutionEngine, Jit, JitBuilder, JitSymbolFlags};
use lyftar::memory_buffer::MemoryBuffer;
use lyftar::passes::PassOptions;
use lyftar::target::{
    CodeGenerationFileType, CodeGenerationOptimizationLevel, CodeModel, RelocationMode, Target, TargetMachine,
    default_target_triple, host_cpu_features, host_cpu_name,
};
use lyftar::{Bits, Builder, Context, Function, Integer, OwnedModule, Type, Value, with_context};

type I1 = Integer<Bits<1>>;
type I8 = Integer<Bits<8>>;
type I64 = Integer<Bits<64>>;

/// Only one program runs at a time, as the collector says.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

const LIST_LENGTH: u64 = 12;

/// How a strategy finds roots, which is what shapes the program.
#[derive(Clone, Copy, PartialEq)]
enum Roots {
    /// Pointers in address space 1 stay as plain values, and the pass says what is live.
    Statepoints,
    /// Pointers live in stack slots, marked with `llvm.gcroot`, and are read again after each call.
    Slots,
}

#[derive(Clone, Copy)]
struct Strategy {
    name: &'static str,
    roots: Roots,
    address_space: u32,
}

const STATEPOINT_EXAMPLE: Strategy =
    Strategy { name: "statepoint-example", roots: Roots::Statepoints, address_space: 1 };
const CORECLR: Strategy = Strategy { name: "coreclr", roots: Roots::Statepoints, address_space: 1 };
const SHADOW_STACK: Strategy = Strategy { name: "shadow-stack", roots: Roots::Slots, address_space: 0 };
#[cfg(any(target_vendor = "apple", target_arch = "x86_64"))]
const OCAML: Strategy = Strategy { name: "ocaml", roots: Roots::Slots, address_space: 0 };

/// A strategy written in Rust, in the test, with no C++.
struct AddressSpaceFive;

impl GarbageCollectionStrategy for AddressSpaceFive {
    const NAME: &'static str = "moving-address-space-five";
    const DESCRIPTION: &'static str = "manages the pointers in address space 5";

    fn manages_pointer(&self, pointer_type: Type<'_>) -> Option<bool> {
        Some(pointer_type.address_space() == 5)
    }
}

const WRITTEN_IN_RUST: Strategy =
    Strategy { name: AddressSpaceFive::NAME, roots: Roots::Statepoints, address_space: 5 };

#[derive(Clone, Copy, PartialEq)]
enum Optimization {
    Off,
    On,
}

/// The runtime functions, and what every function in the program shares.
struct Program<'context, 'module> {
    context: &'module Context<'context>,
    module: &'module lyftar::Module<'context>,
    strategy: Strategy,
    optimization: Optimization,
    pointer: Type<'context>,
    i64_type: Type<'context, I64>,
    allocate: Value<'context, 'module, Function>,
    collect: Value<'context, 'module, Function>,
    report: Value<'context, 'module, Function>,
    report_pointer: Value<'context, 'module, Function>,
}

/// A pointer variable in one function: an SSA value, or a slot it is read from each time.
#[derive(Clone, Copy)]
enum Variable<'context, 'module> {
    Value(Value<'context, 'module>),
    Slot(Value<'context, 'module>),
}

impl<'context, 'module> Program<'context, 'module> {
    fn new(
        context: &'module Context<'context>,
        module: &'module lyftar::Module<'context>,
        strategy: Strategy,
        optimization: Optimization,
    ) -> Self {
        let pointer = context.pointer_type(strategy.address_space).to_dynamic();
        let i64_type = context.get_type::<I64>();
        let i64_dynamic = i64_type.to_dynamic();
        let void = context.void_type();
        let allocate =
            module.add_function("gc_allocate", context.function_type(pointer, &[i64_dynamic, i64_dynamic], false));
        let collect = module.add_function("gc_collect", context.function_type(void, &[], false));
        let report = module.add_function("gc_report", context.function_type(void, &[i64_dynamic], false));
        let report_pointer = module.add_function("gc_report_pointer", context.function_type(void, &[pointer], false));
        // These two can't collect, so a statepoint needn't wrap them.
        report.add_target_dependent_attribute("gc-leaf-function", "");
        report_pointer.add_target_dependent_attribute("gc-leaf-function", "");
        Program {
            context,
            module,
            strategy,
            optimization,
            pointer,
            i64_type,
            allocate,
            collect,
            report,
            report_pointer,
        }
    }

    /// A function using the strategy, with a frame pointer for the collector to walk, and a builder at its entry.
    fn function(
        &self,
        name: &str,
        return_type: Type<'context>,
        parameters: &[Type<'context>],
    ) -> (Value<'context, 'module, Function>, Builder<'context, 'module>) {
        let function = self.module.add_function(name, self.context.function_type(return_type, parameters, false));
        function.set_garbage_collector(self.strategy.name);
        function.add_target_dependent_attribute("frame-pointer", "all");
        let builder = Builder::at_end_of(function.append_basic_block("entry"));
        (function, builder)
    }

    /// A pointer variable starting with this value. For slots, this has to be called in the entry block.
    fn variable(
        &self,
        builder: &Builder<'context, 'module>,
        value: Value<'context, 'module>,
    ) -> Variable<'context, 'module> {
        match self.strategy.roots {
            Roots::Statepoints => Variable::Value(value),
            Roots::Slots => {
                let slot = builder.stack_allocate(self.pointer, "root").to_dynamic();
                builder.garbage_collection_root(slot, self.pointer.constant_null());
                builder.store(value, slot);
                Variable::Slot(slot)
            }
        }
    }

    fn read(
        &self,
        builder: &Builder<'context, 'module>,
        variable: Variable<'context, 'module>,
    ) -> Value<'context, 'module> {
        match variable {
            Variable::Value(value) => value,
            Variable::Slot(slot) => builder.load(self.pointer, slot, "object").to_dynamic(),
        }
    }

    fn field(
        &self,
        builder: &Builder<'context, 'module>,
        object: Value<'context, 'module>,
        index: u64,
    ) -> Value<'context, 'module> {
        let offset = self.i64_type.constant_integer(16 + index * 8, false);
        builder.get_element_pointer(self.context.get_type::<I8>().to_dynamic(), object, &[offset], "field").to_dynamic()
    }

    fn number(&self, value: u64) -> Value<'context, 'module> {
        self.i64_type.constant_integer(value, false).to_dynamic()
    }

    fn allocate(&self, builder: &Builder<'context, 'module>, pointers: u64, data: u64) -> Value<'context, 'module> {
        builder.call(self.allocate, &[self.number(pointers), self.number(data)], "new").to_dynamic()
    }

    /// Stores a pointer in an object: through `llvm.gcwrite`, for a strategy of roots.
    fn write_pointer(
        &self,
        builder: &Builder<'context, 'module>,
        value: Value<'context, 'module>,
        object: Value<'context, 'module>,
        index: u64,
    ) {
        let field = self.field(builder, object, index);
        match self.strategy.roots {
            Roots::Statepoints => {
                builder.store(value, field);
            }
            Roots::Slots => builder.garbage_collection_write(value, object, field),
        }
    }

    /// Loads a pointer from an object: through `llvm.gcread`, for a strategy of roots.
    fn read_pointer(
        &self,
        builder: &Builder<'context, 'module>,
        object: Value<'context, 'module>,
        index: u64,
    ) -> Value<'context, 'module> {
        let field = self.field(builder, object, index);
        match self.strategy.roots {
            Roots::Statepoints => builder.load(self.pointer, field, "pointer").to_dynamic(),
            Roots::Slots => builder.garbage_collection_read(object, field, "pointer").to_dynamic(),
        }
    }

    /// Statepoints written by hand: it allocates through one with a `gc.result`, then collects through another that says what is
    /// live, with a deoptimization value, and uses what `gc.relocate` gives. The numbers add up to 18 only if each of the three was
    /// rewritten. Not optimized, one of them is held in a stack allocation in the live list instead: the optimizer drops a live
    /// value nothing relocates, and then doesn't keep the allocation, as `StatepointCall` warns.
    fn written_by_hand(&self) -> Value<'context, 'module, Function> {
        let (function, builder) = self.function("written_by_hand", self.i64_type.to_dynamic(), &[]);
        // `rewrite-statepoints-for-gc` adds nothing to a statepoint it finds, so inlined into a function it rewrites, what that
        // function holds wouldn't be in this one's live list.
        function.add_attribute(
            AttributeLocation::Function,
            self.context.enum_attribute(AttributeKind::for_name("noinline").expect("an attribute"), 0),
        );
        let uses_stack_allocation = self.optimization == Optimization::Off;
        let slot = builder.stack_allocate(self.pointer, "slot").to_dynamic();
        let call = |callee: Value<'context, 'module, Function>,
                    arguments: &[Value<'context, 'module>],
                    live: &[Value<'context, 'module>],
                    deoptimization: &[Value<'context, 'module>],
                    transition: &[Value<'context, 'module>]| {
            builder.garbage_collection_statepoint(
                &StatepointCall {
                    id: 45,
                    patch_size_in_bytes: 0,
                    function_type: callee.value_type(),
                    callee: callee.to_dynamic(),
                    flags: if transition.is_empty() { StatepointFlags::NONE } else { StatepointFlags::TRANSITION },
                    arguments,
                    transition,
                    deoptimization,
                    live,
                },
                "statepoint",
            )
        };
        let (zero, two, one) = (self.number(0), self.number(2), self.number(1));
        let held = call(self.allocate, &[zero, one], &[], &[], &[]);
        let held = builder.garbage_collection_result(held, "held");
        builder.store(self.number(7), self.field(&builder, held, 0));
        builder.store(held, slot);
        let kept = if uses_stack_allocation { slot } else { held };
        let made = call(self.allocate, &[zero, two], &[kept], &[], &[]);
        let object = builder.garbage_collection_result(made, "object");
        let held = if uses_stack_allocation { held } else { builder.garbage_collection_relocate(made, 0, 0, "held") };
        builder.store(self.number(5), self.field(&builder, object, 0));
        builder.store(self.number(6), self.field(&builder, object, 1));
        // A pointer that isn't the object's start, and that LLVM can't work out from it: which field depends on a value it can't see.
        let which = builder.load(self.i64_type, self.field(&builder, object, 0), "which");
        let offset = builder.integer_multiply(which, self.i64_type.constant_integer(0, false), "offset");
        let start = self.field(&builder, object, 1);
        let derived = builder
            .get_element_pointer(self.context.get_type::<I8>().to_dynamic(), start, &[offset], "derived")
            .to_dynamic();
        // Only x86 can make code for a transition.
        let thirteen = [self.number(13)];
        let transition: &[Value<'context, 'module>] = if cfg!(target_arch = "x86_64") { &thirteen } else { &[] };
        let third = if uses_stack_allocation { slot } else { held };
        let collected = call(self.collect, &[], &[object, derived, third], &[self.number(9)], transition);
        let object = builder.garbage_collection_relocate(collected, 0, 0, "object");
        let derived = builder.garbage_collection_relocate(collected, 0, 1, "derived");
        let held = if uses_stack_allocation {
            builder.load(self.pointer, slot, "held").to_dynamic()
        } else {
            builder.garbage_collection_relocate(collected, 2, 2, "held")
        };
        let five = builder.load(self.i64_type, self.field(&builder, object, 0), "five");
        let six = builder.load(self.i64_type, derived, "six");
        let seven = builder.load(self.i64_type, self.field(&builder, held, 0), "seven");
        builder.return_value(builder.integer_add(builder.integer_add(five, six, ""), seven, "total"));
        function
    }

    /// `build(n)`: a list of `n`, built from the end, each new cell allocated with the rest of the list held across it.
    fn build_list(&self) -> Value<'context, 'module, Function> {
        let i64_dynamic = self.i64_type.to_dynamic();
        let (function, builder) = self.function("build_list", self.pointer, &[i64_dynamic]);
        let null = self.pointer.constant_null().to_dynamic();
        let count = function.parameter(0);
        let more = function.append_basic_block("more");
        let end = function.append_basic_block("end");
        let is_empty = builder.integer_compare(
            IntegerPredicate::Equal,
            count.checked_cast::<I64>(),
            self.i64_type.constant_integer(0, false),
            "is_empty",
        );
        builder.conditional_branch(is_empty, end, more);
        builder.position_at_end(end);
        builder.return_value(null);
        builder.position_at_end(more);
        let smaller =
            builder.integer_subtract(count.checked_cast::<I64>(), self.i64_type.constant_integer(1, false), "smaller");
        // The slot has to be in the entry block.
        let tail = match self.strategy.roots {
            Roots::Statepoints => None,
            Roots::Slots => {
                let first = function.first_basic_block().expect("the entry block");
                let terminator = first.terminator().expect("its branch");
                builder.position_before(terminator);
                let slot = self.variable(&builder, null);
                builder.position_at_end(more);
                Some(slot)
            }
        };
        let rest = builder.call(function, &[smaller.to_dynamic()], "rest").to_dynamic();
        let tail = match tail {
            Some(slot) => {
                if let Variable::Slot(address) = slot {
                    builder.store(rest, address);
                }
                slot
            }
            None => Variable::Value(rest),
        };
        let cell = self.allocate(&builder, 1, 1);
        let rest_now = self.read(&builder, tail);
        self.write_pointer(&builder, rest_now, cell, 0);
        builder.store(count, self.field(&builder, cell, 1));
        builder.return_value(cell);
        function
    }

    /// `sum(list)`: reads the next cell, recurses, collects, and only then reads this cell's number, so every frame holds a list
    /// cell across a recursive call and a collection.
    fn sum_list(&self) -> Value<'context, 'module, Function> {
        let (function, builder) = self.function("sum_list", self.i64_type.to_dynamic(), &[self.pointer]);
        let cell = self.variable(&builder, function.parameter(0));
        let more = function.append_basic_block("more");
        let end = function.append_basic_block("end");
        let is_null = builder.is_null(self.read(&builder, cell), "is_null");
        builder.conditional_branch(is_null, end, more);
        builder.position_at_end(end);
        builder.return_value(self.i64_type.constant_integer(0, false));
        builder.position_at_end(more);
        let object = self.read(&builder, cell);
        let next = self.read_pointer(&builder, object, 0);
        let rest = builder.call(function, &[next], "rest").checked_cast::<I64>();
        builder.call(self.collect, &[] as &[Value<'context, 'module>], "");
        let object = self.read(&builder, cell);
        let number = builder.load(self.i64_type, self.field(&builder, object, 1), "number");
        builder.return_value(builder.integer_add(rest, number, "total"));
        function
    }

    /// Three objects held across each other's allocations and a collection, reporting where the first one is on each side.
    fn hold_many(&self) -> Value<'context, 'module, Function> {
        let (function, builder) = self.function("hold_many", self.i64_type.to_dynamic(), &[]);
        let null = self.pointer.constant_null().to_dynamic();
        let first_variable = self.variable(&builder, null);
        let second_variable = self.variable(&builder, null);
        let third_variable = self.variable(&builder, null);
        let set = |variable: Variable<'context, 'module>, value: Value<'context, 'module>| match variable {
            Variable::Slot(slot) => {
                builder.store(value, slot);
                variable
            }
            Variable::Value(_) => Variable::Value(value),
        };
        let first_variable = set(first_variable, self.allocate(&builder, 0, 1));
        let object = self.read(&builder, first_variable);
        builder.store(self.number(1), self.field(&builder, object, 0));
        let second_variable = set(second_variable, self.allocate(&builder, 0, 1));
        let object = self.read(&builder, second_variable);
        builder.store(self.number(2), self.field(&builder, object, 0));
        let third_variable = set(third_variable, self.allocate(&builder, 1, 1));
        let object = self.read(&builder, third_variable);
        let held = self.read(&builder, first_variable);
        builder.store(held, self.field(&builder, object, 0));
        builder.store(self.number(3), self.field(&builder, object, 1));
        builder.call(self.report_pointer, &[self.read(&builder, first_variable)], "");
        builder.call(self.collect, &[] as &[Value<'context, 'module>], "");
        builder.call(self.report_pointer, &[self.read(&builder, first_variable)], "");
        let load_number = |variable, index| {
            let object = self.read(&builder, variable);
            builder.load(self.i64_type, self.field(&builder, object, index), "number")
        };
        let third = self.read(&builder, third_variable);
        let through_third = builder.load(self.pointer, self.field(&builder, third, 0), "through_third").to_dynamic();
        let through = builder.load(self.i64_type, self.field(&builder, through_third, 0), "through");
        let (one, two, three) =
            (load_number(first_variable, 0), load_number(second_variable, 0), load_number(third_variable, 1));
        let total = builder.integer_add(
            builder.integer_add(one, builder.integer_multiply(two, self.i64_type.constant_integer(10, false), ""), ""),
            builder.integer_multiply(three, self.i64_type.constant_integer(100, false), ""),
            "",
        );
        builder.return_value(builder.integer_add(
            total,
            builder.integer_multiply(through, self.i64_type.constant_integer(1000, false), ""),
            "total",
        ));
        function
    }

    /// Walks an object's numbers with a pointer into its middle, collecting at every step. How far each step goes is read from the
    /// object each time, so the optimizer can't work the pointer out from a counter: with statepoints it stays live across each
    /// collection as a derived pointer. With slots it is worked out again from the rooted object.
    fn walk_array(&self) -> Value<'context, 'module, Function> {
        let i64_dynamic = self.i64_type.to_dynamic();
        let (function, builder) = self.function("walk_array", i64_dynamic, &[self.pointer, i64_dynamic]);
        let array = self.variable(&builder, function.parameter(0));
        let count = function.parameter(1).checked_cast::<I64>();
        let entry = function.first_basic_block().expect("the entry block");
        let body = function.append_basic_block("body");
        let done = function.append_basic_block("done");
        let start = self.read(&builder, array);
        let first = self.field(&builder, start, 1);
        builder.branch(body);
        builder.position_at_end(body);
        let index = builder.phi(self.i64_type, "index");
        let sum = builder.phi(self.i64_type, "sum");
        let cursor = (self.strategy.roots == Roots::Statepoints).then(|| builder.phi(self.pointer, "cursor"));
        builder.call(self.collect, &[] as &[Value<'context, 'module>], "");
        let bytes = self.context.get_type::<I8>().to_dynamic();
        let object = self.read(&builder, array);
        let step = builder.load(self.i64_type, self.field(&builder, object, 0), "step");
        let address = match cursor {
            Some(cursor) => cursor.as_value(),
            None => {
                let offset = builder.integer_multiply(index.as_value(), step, "offset");
                let start = self.field(&builder, object, 1);
                builder.get_element_pointer(bytes, start, &[offset], "address").to_dynamic()
            }
        };
        let number = builder.load(self.i64_type, address, "number");
        let new_sum = builder.integer_add(sum.as_value(), number, "new_sum");
        let new_index = builder.integer_add(index.as_value(), self.i64_type.constant_integer(1, false), "new_index");
        if let Some(cursor) = cursor {
            let next = builder.get_element_pointer(bytes, address, &[step], "next").to_dynamic();
            cursor.add_incoming(&[(first, entry), (next, body)]);
        }
        let again = builder.integer_compare(IntegerPredicate::UnsignedLessThan, new_index, count, "again");
        builder.conditional_branch(again, body, done);
        index.add_incoming(&[(self.i64_type.constant_integer(0, false), entry), (new_index, body)]);
        sum.add_incoming(&[(self.i64_type.constant_integer(0, false), entry), (new_sum, body)]);
        builder.position_at_end(done);
        builder.return_value(new_sum);
        function
    }

    /// An object that goes through a collection and comes back as a return value.
    fn make_after_collecting(&self) -> Value<'context, 'module, Function> {
        let (function, builder) = self.function("make_after_collecting", self.pointer, &[]);
        let null = self.pointer.constant_null().to_dynamic();
        let holder = self.variable(&builder, null);
        let made = self.allocate(&builder, 0, 1);
        builder.store(self.number(77), self.field(&builder, made, 0));
        let holder = match holder {
            Variable::Slot(slot) => {
                builder.store(made, slot);
                holder
            }
            Variable::Value(_) => Variable::Value(made),
        };
        builder.call(self.collect, &[] as &[Value<'context, 'module>], "");
        builder.return_value(self.read(&builder, holder));
        function
    }

    /// What the second translation unit has: it holds a list cell across a collection, and gives back its number.
    fn other_unit(&self) {
        let (function, builder) = self.function("other_unit", self.i64_type.to_dynamic(), &[self.pointer]);
        let cell = self.variable(&builder, function.parameter(0));
        builder.call(self.collect, &[] as &[Value<'context, 'module>], "");
        let object = self.read(&builder, cell);
        builder.return_value(builder.load(self.i64_type, self.field(&builder, object, 1), "number"));
    }

    /// What `place-safepoints` inlines: this collector collects at every poll.
    fn safepoint_poll(&self) {
        let poll = self
            .module
            .add_function("gc.safepoint_poll", self.context.function_type(self.context.void_type(), &[], false));
        // Each translation unit has one, and the optimizer would delete one that could go.
        poll.set_linkage(Linkage::WeakOneDefinitionRule);
        let builder = Builder::at_end_of(poll.append_basic_block("entry"));
        builder.call(self.collect, &[] as &[Value<'context, 'module>], "");
        builder.return_void();
    }

    fn program(&self) {
        let build_list = self.build_list();
        let sum_list = self.sum_list();
        let hold_many = self.hold_many();
        let walk_array = self.walk_array();
        let make_after_collecting = self.make_after_collecting();
        let by_hand = (self.strategy.roots == Roots::Statepoints).then(|| self.written_by_hand());
        let statistics = self
            .module
            .add_function("gc_report_statistics", self.context.function_type(self.context.void_type(), &[], false));
        statistics.add_target_dependent_attribute("gc-leaf-function", "");
        let i64_dynamic = self.i64_type.to_dynamic();
        let (run, builder) = self.function("program", self.context.void_type(), &[]);
        let null = self.pointer.constant_null().to_dynamic();
        let list = self.variable(&builder, null);
        let array = self.variable(&builder, null);
        let list = match list {
            Variable::Slot(slot) => {
                builder.store(builder.call(build_list, &[self.number(LIST_LENGTH)], "list"), slot);
                list
            }
            Variable::Value(_) => {
                Variable::Value(builder.call(build_list, &[self.number(LIST_LENGTH)], "list").to_dynamic())
            }
        };
        let total = builder.call(sum_list, &[self.read(&builder, list)], "total");
        builder.call(self.report, &[total.to_dynamic()], "");
        builder.call(self.report_pointer, &[self.read(&builder, list)], "");
        let many = builder.call(hold_many, &[] as &[Value<'context, 'module>], "many");
        builder.call(self.report, &[many.to_dynamic()], "");
        let made = self.allocate(&builder, 0, 9);
        let array = match array {
            Variable::Slot(slot) => {
                builder.store(made, slot);
                array
            }
            Variable::Value(_) => Variable::Value(made),
        };
        let object = self.read(&builder, array);
        builder.store(self.number(8), self.field(&builder, object, 0));
        for index in 0..8 {
            let object = self.read(&builder, array);
            builder.store(self.number(index + 1), self.field(&builder, object, index + 1));
        }
        let walked = builder.call(walk_array, &[self.read(&builder, array), self.number(8)], "walked");
        builder.call(self.report, &[walked.to_dynamic()], "");
        let returned = builder.call(make_after_collecting, &[] as &[Value<'context, 'module>], "returned").to_dynamic();
        let number = builder.load(self.i64_type, self.field(&builder, returned, 0), "number");
        builder.call(self.report, &[number.to_dynamic()], "");
        let other_unit_type = self.context.function_type(self.i64_type.to_dynamic(), &[self.pointer], false);
        let other_unit = self.module.add_function("other_unit", other_unit_type);
        let from_other_unit = builder.call(other_unit, &[self.read(&builder, list)], "from_other_unit");
        builder.call(self.report, &[from_other_unit.to_dynamic()], "");
        if let Some(by_hand) = by_hand {
            let total = builder.call(by_hand, &[] as &[Value<'context, 'module>], "by_hand");
            builder.call(self.report, &[total.to_dynamic()], "");
        }
        builder.call(self.report_pointer, &[self.read(&builder, list)], "");
        builder.call(statistics, &[] as &[Value<'context, 'module>], "");
        builder.return_void();
        let _ = i64_dynamic;

        // What says how to find roots, and starts it: not garbage collected itself, so a walk stops here.
        let void = self.context.void_type();
        let start = self.module.add_function("program_start", self.context.function_type(void, &[], false));
        let builder = Builder::at_end_of(start.append_basic_block("entry"));
        match (self.strategy.roots, self.strategy.name) {
            (Roots::Statepoints, _) => {
                let say = self.module.add_function("gc_use_stack_maps", self.context.function_type(void, &[], false));
                builder.call(say, &[] as &[Value<'context, 'module>], "");
            }
            (Roots::Slots, "shadow-stack") => {
                let plain = self.context.pointer_type(0).to_dynamic();
                // Defined here, since a JIT can't find one LLVM makes up in each translation unit as it makes code.
                let head = self.module.add_global("llvm_gc_root_chain", plain);
                head.set_initializer(self.context.pointer_type(0).constant_null());
                let say =
                    self.module.add_function("gc_use_shadow_stack", self.context.function_type(void, &[plain], false));
                builder.call(say, &[head.to_dynamic()], "");
            }
            (Roots::Slots, _) => {
                let plain = self.context.pointer_type(0).to_dynamic();
                let say =
                    self.module.add_function("gc_use_frame_table", self.context.function_type(void, &[plain], false));
                for name in ["camlProgram__frametable", "camlOther__frametable"] {
                    let table = self.module.add_global(name, self.context.get_type::<I1>().to_dynamic());
                    builder.call(say, &[table.to_dynamic()], "");
                }
            }
        }
        builder.call(run, &[] as &[Value<'context, 'module>], "");
        builder.return_void();
    }
}

/// Whether `place-safepoints` adds polls, which it only does for `statepoint-example` and `coreclr`.
#[derive(Clone, Copy, PartialEq)]
enum Polls {
    No,
    Yes,
}

/// A translation unit of the program, through the pipeline this strategy needs, for this machine.
fn unit<'context>(
    context: &mut Context<'context>,
    machine: &TargetMachine,
    name: &str,
    build: impl FnOnce(&Program<'context, '_>),
    strategy: Strategy,
    optimization: Optimization,
    polls: Polls,
) -> OwnedModule<'context> {
    let mut owned = context.create_module(name);
    {
        let module = context.module(&owned);
        module.set_target_machine(machine);
        let program = Program::new(context, module, strategy, optimization);
        if polls == Polls::Yes {
            program.safepoint_poll();
        }
        build(&program);
        assert_eq!(module.verify(), Ok(()), "{}", module.print_to_string());
    }
    let optimizations = if optimization == Optimization::On { "default<O2>" } else { "" };
    let placement = if polls == Polls::Yes { SafepointPlacement::PlacePolls } else { SafepointPlacement::CallsOnly };
    let pipeline = match strategy.roots {
        Roots::Statepoints => statepoint_pipeline(optimizations, placement),
        Roots::Slots => optimizations.to_string(),
    };
    if !pipeline.is_empty() {
        context
            .exclusive_module(&mut owned)
            .run_passes(&pipeline, Some(machine), &PassOptions::default())
            .expect("the pipeline");
    }
    owned
}

fn main_unit<'context>(context: &mut Context<'context>, machine: &TargetMachine, way: &Way) -> OwnedModule<'context> {
    unit(context, machine, "Program", |program| program.program(), way.strategy, way.optimization, way.polls)
}

fn other_unit<'context>(context: &mut Context<'context>, machine: &TargetMachine, way: &Way) -> OwnedModule<'context> {
    unit(context, machine, "Other", |program| program.other_unit(), way.strategy, way.optimization, way.polls)
}

fn emit<'context>(
    context: &mut Context<'context>,
    machine: &TargetMachine,
    mut owned: OwnedModule<'context>,
) -> Vec<u8> {
    machine
        .emit_to_memory_buffer(context.exclusive_module(&mut owned), CodeGenerationFileType::Object)
        .expect("code generation")
        .as_bytes()
        .to_vec()
}

/// How one run is compiled.
#[derive(Clone, Copy)]
struct Way {
    strategy: Strategy,
    optimization: Optimization,
    polls: Polls,
}

fn machine_for(optimization: Optimization) -> TargetMachine {
    machine_with(optimization, RelocationMode::PositionIndependentCode, CodeModel::Default)
}

/// RuntimeDyld can't reach between objects more than 128 MiB apart on AArch64, which a call in the small code model needs, so
/// what goes through it uses the large one, as `ExecutionEngine` does by default. Position-independent code on x86-64 ELF, and all
/// code on AArch64 ELF, reaches the personality function through a weak `DW.ref` symbol the module didn't have, which Lyftar's
/// RuntimeDyld layer has to claim.
fn runtime_dyld_machine(optimization: Optimization) -> TargetMachine {
    let code_model = if cfg!(target_arch = "aarch64") { CodeModel::Large } else { CodeModel::Default };
    machine_with(optimization, RelocationMode::PositionIndependentCode, code_model)
}

fn machine_with(optimization: Optimization, relocation: RelocationMode, code_model: CodeModel) -> TargetMachine {
    let level = if optimization == Optimization::On {
        CodeGenerationOptimizationLevel::Default
    } else {
        CodeGenerationOptimizationLevel::None
    };
    let triple = default_target_triple();
    Target::from_triple(&triple).expect("this machine's target").create_target_machine(
        &triple,
        &host_cpu_name(),
        &host_cpu_features(),
        level,
        relocation,
        code_model,
    )
}

/// What every run has to say. The object has to have moved at each collection, and be in the current space after it.
/// Says how many collections there were.
fn check_output(output: &str, what: &str, way: &Way) -> u64 {
    let statepoints = way.strategy.roots == Roots::Statepoints;
    let values: Vec<i64> = output
        .lines()
        .filter_map(|line| line.strip_prefix("value "))
        .map(|value| value.parse().expect("a number"))
        .collect();
    let list_total = (LIST_LENGTH * (LIST_LENGTH + 1) / 2).cast_signed();
    let mut expected = vec![list_total, 1321, 36, 77, LIST_LENGTH.cast_signed()];
    if statepoints {
        expected.push(18);
    }
    assert_eq!(values, expected, "{what}: {output}");
    let objects: Vec<&str> = output.lines().filter(|line| line.starts_with("object at")).collect();
    assert_eq!(objects.len(), 4, "{what}: {output}");
    for line in &objects {
        assert!(line.ends_with("in the current space: true"), "{what}: {line}");
    }
    let address = |line: &str| line.split_whitespace().nth(2).expect("an address").to_string();
    assert_ne!(
        address(objects[1]),
        address(objects[2]),
        "{what}: the first object moved in the collection between: {output}"
    );
    assert_ne!(address(objects[0]), address(objects[3]), "{what}: the list moved: {output}");
    let statistics = output.lines().find(|line| line.starts_with("collections ")).expect("the statistics");
    let numbers: Vec<u64> = statistics.split([',', ' ']).filter_map(|word| word.parse().ok()).collect();
    let [collections, moved, rewritten, derived, frames, tables] = numbers[..] else {
        panic!("{what}: {statistics}");
    };
    assert!(collections > LIST_LENGTH, "{what}: {statistics}");
    assert!(moved > collections, "{what}: {statistics}");
    assert!(rewritten > collections, "{what}: {statistics}");
    assert!(frames > collections, "{what}: {statistics}");
    assert_eq!(derived > 0, statepoints, "{what}: {statistics}");
    // Two translation units give two tables, in one section or two.
    assert_eq!(tables, if statepoints { 2 } else { 0 }, "{what}: {statistics}");
    collections
}

fn temporary_directory(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("lyftar-collection-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("a temporary directory");
    directory
}

/// A linked program: what Lyftar compiled, and the collector, with no LLVM. Stripping what nothing refers to would drop the stack
/// map, so `link-dead-code` keeps it.
fn run_linked(way: &Way) -> String {
    // A new path each time, since cargo only links again when the arguments change, and it can't see inside the objects.
    static RUNS: AtomicU32 = AtomicU32::new(0);
    let directory = temporary_directory(&format!("{}-{}", way.strategy.name, RUNS.fetch_add(1, Ordering::Relaxed)));
    let (program_object, other_object) = (directory.join("program.o"), directory.join("other.o"));
    let (program_bytes, other_bytes) = with_context(|context| {
        let machine = machine_for(way.optimization);
        let program = main_unit(context, &machine, way);
        let other = other_unit(context, &machine, way);
        (emit(context, &machine, program), emit(context, &machine, other))
    });
    std::fs::write(&program_object, program_bytes).expect("writing the object");
    std::fs::write(&other_object, other_bytes).expect("writing the object");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut build = Command::new(env!("CARGO"));
    build
        .current_dir(root)
        .args(["rustc", "--package", "lyftar-test-collector", "--target-dir", "target/collector"])
        .arg("--")
        .args(["-C", "link-dead-code"])
        .arg("-C")
        .arg(format!("link-arg={}", program_object.display()))
        .arg("-C")
        .arg(format!("link-arg={}", other_object.display()));
    // ELF's stack map section is read-only and holds addresses, which a position-independent program has to relocate when it loads.
    if cfg!(target_os = "linux") {
        build.args(["-C", "link-arg=-Wl,-z,notext"]);
    }
    let built = build.output().expect("running cargo");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let program = root.join("target/collector/debug/lyftar-test-collector");
    let ran = Command::new(&program).output().expect("running the program");
    assert!(
        ran.status.success(),
        "{}: {:?}: {}{}",
        way.strategy.name,
        ran.status,
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
    check_does_not_link_llvm(&program);
    let _ = std::fs::remove_dir_all(directory);
    String::from_utf8_lossy(&ran.stdout).into_owned()
}

fn check_does_not_link_llvm(program: &Path) {
    let tool = if cfg!(target_os = "macos") { ("otool", "-L") } else { ("ldd", "--") };
    let output = Command::new(tool.0).arg(tool.1).arg(program).output().expect("listing what it links");
    let linked = String::from_utf8_lossy(&output.stdout).to_lowercase();
    assert!(output.status.success());
    assert!(!linked.contains("libllvm") && !linked.contains("liblto"), "{linked}");
}

/// The runtime's functions, by name, for a JIT to bind.
fn runtime_functions() -> [(&'static str, u64); 9] {
    [
        ("gc_allocate", collector::gc_allocate as *const () as u64),
        ("gc_collect", collector::gc_collect as *const () as u64),
        ("gc_report", collector::gc_report as *const () as u64),
        ("gc_report_pointer", collector::gc_report_pointer as *const () as u64),
        ("gc_report_statistics", collector::gc_report_statistics as *const () as u64),
        ("gc_use_stack_maps", collector::gc_use_stack_maps as *const () as u64),
        ("gc_use_shadow_stack", collector::gc_use_shadow_stack as *const () as u64),
        ("gc_use_frame_table", collector::gc_use_frame_table as *const () as u64),
        ("unused", 0),
    ]
}

fn run_jit(way: &Way, builder: JitBuilder) -> String {
    let jit = builder.build().expect("a JIT");
    let library = jit.main_dylib();
    let names: Vec<_> = runtime_functions().iter().map(|(name, _)| jit.mangle_and_intern(name)).collect();
    let flags = JitSymbolFlags { exported: true, callable: true, ..JitSymbolFlags::default() };
    let symbols: Vec<_> =
        names.iter().zip(runtime_functions()).map(|(name, (_, address))| (name, address, flags)).collect();
    library.define_absolute_symbols(&symbols).expect("defining the runtime");
    if way.strategy.name == "ocaml" {
        // Its frame table is a symbol the module doesn't declare, which ORC refuses in a module, and takes in an object.
        let (program, other) = with_context(|context| {
            let machine = runtime_dyld_machine(way.optimization);
            let program = main_unit(context, &machine, way);
            let other = other_unit(context, &machine, way);
            (emit(context, &machine, program), emit(context, &machine, other))
        });
        jit.add_object_file(library, MemoryBuffer::from_bytes(&program, "program")).expect("adding the program");
        jit.add_object_file(library, MemoryBuffer::from_bytes(&other, "other")).expect("adding the other unit");
    } else {
        jit.add_module(library, |context| main_unit(context, &machine_for(way.optimization), way))
            .expect("adding the program");
        jit.add_module(library, |context| other_unit(context, &machine_for(way.optimization), way))
            .expect("adding the other unit");
    }
    let start = jit.lookup("program_start").expect("the program");
    // Looking this up links what the program calls, so both units' stack maps are there before it runs.
    jit.lookup("other_unit").expect("the other unit");
    // SAFETY: this function takes nothing and returns nothing, as built above.
    let start = unsafe { std::mem::transmute::<usize, extern "C" fn()>(start as usize) };
    let sections = jit.stack_map_sections();
    if way.strategy.roots == Roots::Statepoints {
        assert_eq!(sections.len(), 2, "the JIT kept both units' stack maps");
    }
    collector::reset(sections);
    start();
    collector::take_output()
}

fn run_execution_engine(way: &Way) -> String {
    with_context(|context| {
        let machine = machine_for(way.optimization);
        let program = main_unit(context, &machine, way);
        let other = other_unit(context, &machine, way);
        let level = if way.optimization == Optimization::On { 2 } else { 0 };
        let mut engine = ExecutionEngine::jit_compiler(context, program, level).expect("an engine");
        engine.add_module(other);
        // Only declared in the program, since they're in this process. Without this a call to one goes to address 0 on Linux.
        for (name, address) in runtime_functions() {
            if let Some(function) = engine.find_declared_function(name) {
                engine.add_global_mapping(function, address as *mut std::ffi::c_void);
            }
        }
        let sections = engine.stack_map_sections();
        if way.strategy.roots == Roots::Statepoints {
            assert_eq!(sections.len(), 2, "the engine kept both units' stack maps");
        }
        let start = engine.function_address("program_start");
        assert_ne!(start, 0);
        // SAFETY: this function takes nothing and returns nothing, as built above.
        let start = unsafe { std::mem::transmute::<usize, extern "C" fn()>(start as usize) };
        collector::reset(sections);
        start();
        collector::take_output()
    })
}

fn every_way(strategy: Strategy, polls: Polls) {
    let _lock = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for optimization in [Optimization::Off, Optimization::On] {
        let label = if optimization == Optimization::On { "optimized" } else { "not optimized" };
        let way = Way { strategy, optimization, polls };
        let runtime_dyld = || {
            let mut builder = JitBuilder::new();
            builder.use_runtime_dyld();
            builder.set_target_machine(runtime_dyld_machine(optimization));
            builder
        };
        // A poll collects as well, so a program with them has to collect more than one without.
        let without_polls = Way { polls: Polls::No, ..way };
        let least_collections = check_output(&run_jit(&without_polls, runtime_dyld()), "without polls", &without_polls);
        for (name, output) in [
            // JITLink refuses an `ocaml` object on some layouts, as an error (ASSUMPTIONS.md).
            ("through Jit", run_jit(&way, if strategy.name == "ocaml" { runtime_dyld() } else { JitBuilder::new() })),
            ("through Jit with RuntimeDyld", run_jit(&way, runtime_dyld())),
            ("through ExecutionEngine", run_execution_engine(&way)),
            ("linked", run_linked(&way)),
        ] {
            let what = format!("{} {name}, {label}", strategy.name);
            let collections = check_output(&output, &what, &way);
            if polls == Polls::Yes {
                assert!(collections > least_collections, "{what}: {collections} collections, with polls");
            }
        }
    }
}

#[test]
fn decision_45_statepoint_example_moves_objects_and_rewrites_every_root() {
    every_way(STATEPOINT_EXAMPLE, Polls::No);
}

#[test]
fn decision_45_coreclr_moves_objects_and_rewrites_every_root() {
    every_way(CORECLR, Polls::No);
}

#[test]
fn decision_45_shadow_stack_moves_objects_and_rewrites_every_root() {
    every_way(SHADOW_STACK, Polls::No);
}

// trace: D54
// Its frame table can't say which register an offset is from on Linux on ARM, so it runs only where the sign of the offset does.
#[cfg(any(target_vendor = "apple", target_arch = "x86_64"))]
#[test]
fn decision_45_ocaml_moves_objects_and_rewrites_every_root() {
    every_way(OCAML, Polls::No);
}

#[test]
fn decision_45_a_strategy_written_in_rust_moves_objects_and_rewrites_every_root() {
    register_strategy(AddressSpaceFive);
    every_way(WRITTEN_IN_RUST, Polls::No);
}

#[test]
fn decision_45_place_safepoints_polls_collect_and_rewrite_every_root() {
    every_way(STATEPOINT_EXAMPLE, Polls::Yes);
    every_way(CORECLR, Polls::Yes);
}

#[test]
fn decision_45_what_a_tracker_removes_goes_from_the_stack_maps() {
    let jit = Jit::new().expect("a JIT");
    let library = jit.main_dylib();
    let names: Vec<_> = runtime_functions().iter().map(|(name, _)| jit.mangle_and_intern(name)).collect();
    let flags = JitSymbolFlags { exported: true, callable: true, ..JitSymbolFlags::default() };
    let symbols: Vec<_> =
        names.iter().zip(runtime_functions()).map(|(name, (_, address))| (name, address, flags)).collect();
    library.define_absolute_symbols(&symbols).expect("defining the runtime");
    let tracker = library.create_resource_tracker();
    let way = Way { strategy: STATEPOINT_EXAMPLE, optimization: Optimization::Off, polls: Polls::No };
    jit.add_module_with_tracker(&tracker, |context| other_unit(context, &machine_for(way.optimization), &way))
        .expect("adding it");
    assert!(jit.stack_map_sections().is_empty(), "nothing is linked until it is looked up");
    jit.lookup("other_unit").expect("the function");
    assert_eq!(jit.stack_map_sections().len(), 1);
    tracker.remove().expect("removing");
    assert!(jit.stack_map_sections().is_empty());
}
