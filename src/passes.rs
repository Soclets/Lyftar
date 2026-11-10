//! Writing LLVM passes in Rust, with no C++ for the pass author, and running pipelines (D1, D16).
//!
//! A [`ModulePass`] or [`FunctionPass`] runs linked into a compiler through [`Module::run_passes_with`], and through `opt
//! -load-pass-plugin` or `clang -fpass-plugin` through [`pass_plugin!`](crate::pass_plugin). A panic inside a pass aborts the
//! compiler with its message, since the IR may be half rewritten (D3).

use std::cell::Cell;
use std::ffi::{CString, c_char, c_void};
use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::basic_block::BasicBlock;
use crate::error::Error;
use crate::module::{Module, ReplacementToMake, TrackedHandle, ValueToErase};
use crate::raw;
use crate::shim;
use crate::target::TargetMachine;
use crate::text::{c_string, from_owned_message};
use crate::types::Invariant;
use crate::values::{CallBaseMarker, Function, InstructionMarker, Value, ValueMarker};

/// What a pass leaves valid. Lyftar checks a claim of [`PreservedAnalyses::All`] against the IR: a pass that changed something
/// and claimed nothing stops there, because LLVM's later passes would trust analyses that no longer hold (D3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreservedAnalyses {
    /// The pass changed nothing.
    All,
    /// The pass may have changed anything.
    None,
}

/// A pass over a whole module.
pub trait ModulePass {
    /// Runs the pass.
    fn run(&mut self, module: &mut Module<'_>, analyses: &Analyses<'_>) -> PreservedAnalyses;
}

/// A pass over one function at a time. LLVM is in the middle of walking the module's functions, so it can change only this
/// function, which [`FunctionInPass`] enforces.
pub trait FunctionPass {
    /// Runs the pass.
    fn run(&mut self, function: &mut FunctionInPass<'_>, analyses: &Analyses<'_>) -> PreservedAnalyses;
}

/// What a function pass works on: this function, exclusively, and its module to read.
pub struct FunctionInPass<'context> {
    module: Module<'context>,
    function: raw::LLVMValueRef,
}

impl<'context> FunctionInPass<'context> {
    /// The function.
    #[must_use]
    pub fn function(&self) -> Value<'context, '_, Function> {
        // SAFETY: LLVM gave the pass this function; it lives as long as this borrow, since only `&mut self` can erase.
        unsafe { Value::from_raw(self.function) }
    }

    // trace: D34
    /// Chooses a replacement for [`FunctionInPass::replace`], of an instruction or argument of this function.
    ///
    /// # Panics
    /// As [`Module::replacement_to_make`], and if `old` isn't in this function (D3).
    #[must_use]
    #[track_caller]
    pub fn replacement_to_make<T, U>(
        &self,
        old: Value<'context, '_, T>,
        new: Value<'context, '_, U>,
    ) -> ReplacementToMake {
        // SAFETY: the value is live; the shim only reads it.
        if !unsafe { shim::lyftar_value_is_in_function(old.raw, self.function) } {
            bad_call(format_args!(
                "replacement_to_make: a function pass can only replace in its own function, and {} isn't there",
                old.describe()
            ));
        }
        self.module.replacement_to_make(old, new)
    }

    // trace: D34
    /// Makes these replacements, as [`Module::replace`] does.
    ///
    /// # Panics
    /// As [`Module::replace`], and if an entry wasn't chosen for this function (D3).
    #[track_caller]
    pub fn replace(&mut self, entries: &[ReplacementToMake]) {
        // A stale entry may be dangling, so this asks of it only after `Module::replace` would have refused it.
        self.module.check_replacements_are_current(entries);
        for entry in entries {
            // SAFETY: the entry is current, so its value is live; the shim only reads it.
            if !unsafe { shim::lyftar_value_is_in_function(entry.old, self.function) } {
                bad_call(format_args!(
                    "replace: a function pass can only replace in its own function, and this entry is from elsewhere"
                ));
            }
        }
        self.module.replace(entries);
    }

    /// The function's module, to read and to build in. Erasing from it needs [`FunctionInPass::erase`].
    #[must_use]
    pub fn module(&self) -> &Module<'context> {
        &self.module
    }

    // trace: D11
    /// Chooses an instruction of this function for [`FunctionInPass::erase`].
    ///
    /// # Panics
    /// If it isn't an instruction in this function (D3).
    #[must_use]
    #[track_caller]
    pub fn value_to_erase<T: InstructionMarker>(&self, instruction: Value<'context, '_, T>) -> ValueToErase {
        // SAFETY: the instruction is live for its borrow.
        let parent = unsafe { raw::LLVMGetBasicBlockParent(raw::LLVMGetInstructionParent(instruction.raw)) };
        if parent != self.function {
            bad_call(format_args!(
                "value_to_erase: a function pass can only erase from its own function, and {} isn't there",
                instruction.describe()
            ));
        }
        self.module.value_to_erase(instruction)
    }

    // trace: D11
    /// Chooses a block of this function for [`FunctionInPass::erase`].
    ///
    /// # Panics
    /// If it isn't a block in this function (D3).
    #[must_use]
    #[track_caller]
    pub fn basic_block_to_erase(&self, block: BasicBlock<'context, '_>) -> ValueToErase {
        if block.parent().raw != self.function {
            bad_call(format_args!(
                "basic_block_to_erase: a function pass can only erase from its own function, and %{} isn't there",
                block.name()
            ));
        }
        self.module.basic_block_to_erase(block)
    }

    // trace: D11
    /// Erases these values and blocks together, as [`Module::erase`] does.
    ///
    /// # Panics
    /// As [`Module::erase`] does, and if an entry is outside this function, since LLVM is walking the module's functions
    /// (D3).
    #[track_caller]
    pub fn erase(&mut self, entries: &[ValueToErase]) {
        for entry in entries {
            self.check_in_function("erase", *entry);
        }
        self.module.erase(entries);
    }

    #[track_caller]
    fn check_in_function(&self, operation: &str, entry: ValueToErase) {
        if !self.module.entry_is_current(entry) {
            bad_call(format_args!(
                "{operation}: this entry is stale: it was chosen before something exclusive happened to this module, or it \
                 came from another module"
            ));
        }
        // SAFETY: the entry is current, so it is live and in this module, and the shim only reads it.
        if !unsafe { shim::lyftar_value_is_in_function(entry.raw, self.function) } {
            bad_call(format_args!("{operation}: a function pass can only change its own function"));
        }
    }

    /// A handle on a value that survives changes and says whether the value is still there (D11).
    ///
    /// # Panics
    /// As [`Module::track`] does (D3).
    #[must_use]
    #[track_caller]
    pub fn track<T: ValueMarker>(&self, value: Value<'context, '_, T>) -> TrackedHandle<'context, T> {
        self.module.track(value)
    }

    /// Chooses a call in this function for [`FunctionInPass::inline_call`].
    ///
    /// # Panics
    /// If it isn't in this function (D3).
    #[must_use]
    #[track_caller]
    pub fn call_to_inline<T: CallBaseMarker>(&self, call: Value<'context, '_, T>) -> CallToInline {
        // SAFETY: the call is live for its borrow.
        let parent = unsafe { raw::LLVMGetBasicBlockParent(raw::LLVMGetInstructionParent(call.raw)) };
        if parent != self.function {
            bad_call(format_args!(
                "call_to_inline: a function pass can only inline into its own function, and {} isn't there",
                call.describe()
            ));
        }
        self.module.call_to_inline(call)
    }

    /// Inlines a call, as [`Module::inline_call`] does.
    ///
    /// # Errors
    /// Why LLVM couldn't inline it.
    ///
    /// # Panics
    /// As [`Module::inline_call`] does (D3).
    #[track_caller]
    pub fn inline_call(&mut self, call: CallToInline) -> Result<(), Error> {
        self.check_in_function("inline_call", call.entry);
        self.module.inline_call(call)
    }
}

/// A call chosen for [`Module::inline_call`], with no lifetime, as [`ValueToErase`] has none (D11).
#[derive(Clone, Copy, Debug)]
pub struct CallToInline {
    entry: ValueToErase,
}

impl<'context> Module<'context> {
    /// Chooses a call for [`Module::inline_call`].
    ///
    /// # Panics
    /// If the call belongs to another module (D3).
    #[must_use]
    #[track_caller]
    pub fn call_to_inline<T: CallBaseMarker>(&self, call: Value<'context, '_, T>) -> CallToInline {
        CallToInline { entry: self.value_to_erase(call) }
    }

    // trace: D11
    /// Inlines the called function where this call is, and removes the call. It leaves the called function as it is.
    ///
    /// Inlining splits a block and deletes the call, so it needs exclusive access.
    ///
    /// # Errors
    /// Why LLVM couldn't inline it, such as the callee having no body. The call then stays, and a [`TrackedHandle`] on it
    /// still gives it back.
    ///
    /// # Panics
    /// If the choice is stale, because something exclusive has happened to this module since, or it came from another
    /// module (D3).
    #[track_caller]
    pub fn inline_call(&mut self, call: CallToInline) -> Result<(), Error> {
        if !self.entry_is_current(call.entry) {
            bad_call(format_args!(
                "inline_call: this choice is stale: it was made before something exclusive happened to this module, or \
                 it came from another module"
            ));
        }
        self.bump_generation();
        let mut message = std::ptr::null_mut();
        // SAFETY: the entry is current, so the call is live and in this module; exclusive access means nothing holds a
        // value this could delete; the shim writes a message for the caller to free if it fails.
        if unsafe { shim::lyftar_inline_call(call.entry.raw, &raw mut message) } {
            Ok(())
        } else {
            Err(Error::from_owned_message(message))
        }
    }
}

impl<'context, 'module, R, P> Value<'context, 'module, Function<R, P>> {
    /// Adds a copy of this function to its module, with the same name made unique.
    ///
    /// # Panics
    /// If this function has no body (D3).
    #[must_use]
    #[track_caller]
    pub fn clone_function(self) -> Value<'context, 'module, Function> {
        if self.first_basic_block().is_none() {
            bad_call(format_args!("clone_function: {} has no body", self.name()));
        }
        // SAFETY: just checked; a function handed out by Lyftar is in a module, which owns the copy.
        unsafe { Value::from_raw(shim::lyftar_clone_function(self.raw)) }
    }
}

/// What a pass can ask LLVM's analyses about (D11). Results borrow the module, so one can't be held across a delete, and after
/// anything exclusive happens to the module the next question gets fresh results.
pub struct Analyses<'pass> {
    raw: *mut c_void,
    module: raw::LLVMModuleRef,
    generation: Cell<u64>,
    marker: PhantomData<&'pass ()>,
}

impl Analyses<'_> {
    #[track_caller]
    fn ready<'context>(&self, operation: &str, module: &Module<'context>, function: Value<'context, '_, Function>) {
        // A result for any other module's function would stay in LLVM's cache after that module goes.
        if module.raw != self.module {
            bad_call(format_args!("{operation}: this pass's analyses are for another module"));
        }
        module.check_owns(operation, function);
        if function.first_basic_block().is_none() {
            bad_call(format_args!("{operation}: {} has no body", function.name()));
        }
        // trace: D11
        if !crate::checks::entry_is_current(self.generation.get(), module.generation) {
            // SAFETY: results borrow the module, and something exclusive just needed `&mut Module`, so none is in use.
            unsafe { shim::lyftar_analyses_clear(self.raw) };
            self.generation.set(module.generation);
        }
    }

    /// The function's dominator tree.
    ///
    /// # Panics
    /// If `module` isn't this pass's module, the function isn't in it, or it has no body (D3).
    #[must_use]
    #[track_caller]
    pub fn dominator_tree<'context, 'module>(
        &self,
        module: &'module Module<'context>,
        function: Value<'context, 'module, Function>,
    ) -> DominatorTree<'context, 'module> {
        self.ready("dominator_tree", module, function);
        // SAFETY: just checked; LLVM's analysis manager owns the tree until the module changes, which `'module` rules out.
        let raw = unsafe { shim::lyftar_analyses_dominator_tree(self.raw, function.raw) };
        DominatorTree { raw, function, marker: PhantomData }
    }

    /// The function's loop information.
    ///
    /// # Panics
    /// If `module` isn't this pass's module, the function isn't in it, or it has no body (D3).
    #[must_use]
    #[track_caller]
    pub fn loop_info<'context, 'module>(
        &self,
        module: &'module Module<'context>,
        function: Value<'context, 'module, Function>,
    ) -> LoopInfo<'context, 'module> {
        self.ready("loop_info", module, function);
        // SAFETY: just checked; LLVM's analysis manager owns the information until the module changes, which `'module` rules
        // out.
        let raw = unsafe { shim::lyftar_analyses_loop_info(self.raw, function.raw) };
        LoopInfo { raw, function, marker: PhantomData }
    }
}

/// A function's dominator tree. Blocks added since it was made count as unreachable.
#[derive(Clone, Copy)]
pub struct DominatorTree<'context, 'module> {
    raw: *mut c_void,
    function: Value<'context, 'module, Function>,
    marker: PhantomData<Invariant<'context>>,
}

impl<'context, 'module> DominatorTree<'context, 'module> {
    #[track_caller]
    fn check_block(&self, operation: &str, block: BasicBlock<'context, 'module>) {
        if block.parent() != self.function {
            bad_call(format_args!("{operation}: %{} isn't in this tree's function", block.name()));
        }
    }

    /// The entry block.
    #[must_use]
    pub fn root(&self) -> BasicBlock<'context, 'module> {
        // SAFETY: the tree is live for `'module`, as are its blocks.
        unsafe { BasicBlock::from_raw(shim::lyftar_dominator_tree_root(self.raw)) }
    }

    /// Whether every path to `dominated` goes through `dominating`.
    ///
    /// # Panics
    /// If a block is in another function (D3).
    #[must_use]
    #[track_caller]
    pub fn dominates(
        &self,
        dominating: BasicBlock<'context, 'module>,
        dominated: BasicBlock<'context, 'module>,
    ) -> bool {
        self.check_block("dominates", dominating);
        self.check_block("dominates", dominated);
        // SAFETY: just checked, and the tree is live for `'module`.
        unsafe { shim::lyftar_dominator_tree_block_dominates(self.raw, dominating.raw, dominated.raw) }
    }

    /// As [`DominatorTree::dominates`], but a block doesn't properly dominate itself.
    ///
    /// # Panics
    /// If a block is in another function (D3).
    #[must_use]
    #[track_caller]
    pub fn properly_dominates(
        &self,
        dominating: BasicBlock<'context, 'module>,
        dominated: BasicBlock<'context, 'module>,
    ) -> bool {
        self.check_block("properly_dominates", dominating);
        self.check_block("properly_dominates", dominated);
        // SAFETY: just checked, and the tree is live for `'module`.
        unsafe { shim::lyftar_dominator_tree_block_properly_dominates(self.raw, dominating.raw, dominated.raw) }
    }

    /// Whether a value is available wherever `user` runs.
    ///
    /// # Panics
    /// If `user` is in another function (D3).
    #[must_use]
    #[track_caller]
    pub fn value_dominates<T, U: InstructionMarker>(
        &self,
        definition: Value<'context, 'module, T>,
        user: Value<'context, 'module, U>,
    ) -> bool {
        self.check_block("value_dominates", user.parent_basic_block());
        // SAFETY: just checked, and the tree is live for `'module`.
        unsafe { shim::lyftar_dominator_tree_value_dominates(self.raw, definition.raw, user.raw) }
    }

    /// The block's immediate dominator, if it has one.
    ///
    /// # Panics
    /// If the block is in another function (D3).
    #[must_use]
    #[track_caller]
    pub fn immediate_dominator(&self, block: BasicBlock<'context, 'module>) -> Option<BasicBlock<'context, 'module>> {
        self.check_block("immediate_dominator", block);
        // SAFETY: just checked, and the tree is live for `'module`.
        let found = unsafe { shim::lyftar_dominator_tree_immediate_dominator(self.raw, block.raw) };
        // SAFETY: the tree's own block, live for `'module`.
        (!found.is_null()).then(|| unsafe { BasicBlock::from_raw(found) })
    }

    /// Whether the tree knows the block is reachable from the entry.
    ///
    /// # Panics
    /// If the block is in another function (D3).
    #[must_use]
    #[track_caller]
    pub fn is_reachable(&self, block: BasicBlock<'context, 'module>) -> bool {
        self.check_block("is_reachable", block);
        // SAFETY: just checked, and the tree is live for `'module`.
        unsafe { shim::lyftar_dominator_tree_is_reachable(self.raw, block.raw) }
    }

    /// The blocks this one immediately dominates.
    ///
    /// # Panics
    /// If the block is in another function (D3).
    #[must_use]
    #[track_caller]
    pub fn children(&self, block: BasicBlock<'context, 'module>) -> Vec<BasicBlock<'context, 'module>> {
        self.check_block("children", block);
        // SAFETY: just checked; each index is below the count the tree gave, and its blocks are live for `'module`.
        unsafe {
            (0..shim::lyftar_dominator_tree_child_count(self.raw, block.raw))
                .map(|index| BasicBlock::from_raw(shim::lyftar_dominator_tree_child_at(self.raw, block.raw, index)))
                .collect()
        }
    }
}

/// A function's natural loops. Blocks added since it was made are in no loop.
#[derive(Clone, Copy)]
pub struct LoopInfo<'context, 'module> {
    raw: *mut c_void,
    function: Value<'context, 'module, Function>,
    marker: PhantomData<Invariant<'context>>,
}

impl<'context, 'module> LoopInfo<'context, 'module> {
    /// The innermost loop holding a block.
    ///
    /// # Panics
    /// If the block is in another function (D3).
    #[must_use]
    #[track_caller]
    pub fn loop_for(&self, block: BasicBlock<'context, 'module>) -> Option<Loop<'context, 'module>> {
        if block.parent() != self.function {
            bad_call(format_args!("loop_for: %{} isn't in this function", block.name()));
        }
        // SAFETY: just checked, and the information is live for `'module`.
        let found = unsafe { shim::lyftar_loop_info_loop_for(self.raw, block.raw) };
        (!found.is_null()).then_some(Loop { raw: found, marker: PhantomData })
    }

    /// The outermost loops.
    #[must_use]
    pub fn top_level_loops(&self) -> Vec<Loop<'context, 'module>> {
        // SAFETY: the information is live for `'module`, and each index is below the count it gave.
        unsafe {
            (0..shim::lyftar_loop_info_top_level_count(self.raw))
                .map(|index| Loop { raw: shim::lyftar_loop_info_top_level_at(self.raw, index), marker: PhantomData })
                .collect()
        }
    }
}

/// A natural loop.
#[derive(Clone, Copy, Debug)]
pub struct Loop<'context, 'module> {
    raw: *mut c_void,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl PartialEq for Loop<'_, '_> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

fn optional_block<'context, 'module>(found: raw::LLVMBasicBlockRef) -> Option<BasicBlock<'context, 'module>> {
    // SAFETY: a loop's own block, live for `'module`.
    (!found.is_null()).then(|| unsafe { BasicBlock::from_raw(found) })
}

impl<'context, 'module> Loop<'context, 'module> {
    /// The block control enters the loop through.
    #[must_use]
    pub fn header(&self) -> BasicBlock<'context, 'module> {
        // SAFETY: the loop is live for `'module`, as is its header.
        unsafe { BasicBlock::from_raw(shim::lyftar_loop_header(self.raw)) }
    }

    /// The block just before the header, if the loop has exactly one such block that only leads into the loop.
    #[must_use]
    pub fn preheader(&self) -> Option<BasicBlock<'context, 'module>> {
        // SAFETY: the loop is live for `'module`.
        optional_block(unsafe { shim::lyftar_loop_preheader(self.raw) })
    }

    /// The block that branches back to the header, if there is just one.
    #[must_use]
    pub fn latch(&self) -> Option<BasicBlock<'context, 'module>> {
        // SAFETY: the loop is live for `'module`.
        optional_block(unsafe { shim::lyftar_loop_latch(self.raw) })
    }

    /// The loop around this one.
    #[must_use]
    pub fn parent(&self) -> Option<Loop<'context, 'module>> {
        // SAFETY: the loop is live for `'module`.
        let found = unsafe { shim::lyftar_loop_parent(self.raw) };
        (!found.is_null()).then_some(Loop { raw: found, marker: PhantomData })
    }

    /// How deep this loop is, 1 for an outermost loop.
    #[must_use]
    pub fn depth(&self) -> u32 {
        // SAFETY: the loop is live for `'module`.
        unsafe { shim::lyftar_loop_depth(self.raw) }
    }

    /// Whether a block is in this loop.
    #[must_use]
    pub fn contains(&self, block: BasicBlock<'context, 'module>) -> bool {
        // SAFETY: both are live for `'module`.
        unsafe { shim::lyftar_loop_contains_block(self.raw, block.raw) }
    }

    /// The blocks in this loop, the header first.
    #[must_use]
    pub fn basic_blocks(&self) -> Vec<BasicBlock<'context, 'module>> {
        // SAFETY: the loop is live for `'module`, and each index is below the count it gave.
        unsafe {
            (0..shim::lyftar_loop_block_count(self.raw))
                .map(|index| BasicBlock::from_raw(shim::lyftar_loop_block_at(self.raw, index)))
                .collect()
        }
    }

    /// The loops directly inside this one.
    #[must_use]
    pub fn subloops(&self) -> Vec<Loop<'context, 'module>> {
        // SAFETY: the loop is live for `'module`, and each index is below the count it gave.
        unsafe {
            (0..shim::lyftar_loop_subloop_count(self.raw))
                .map(|index| Loop { raw: shim::lyftar_loop_subloop_at(self.raw, index), marker: PhantomData })
                .collect()
        }
    }

    /// The blocks outside this loop that it can branch to.
    #[must_use]
    pub fn exit_basic_blocks(&self) -> Vec<BasicBlock<'context, 'module>> {
        // SAFETY: the loop is live for `'module`; a null buffer of no room asks how many, and the second call has room for
        // them all; its blocks are live for `'module`.
        unsafe {
            let count = shim::lyftar_loop_exit_blocks(self.raw, std::ptr::null_mut(), 0);
            let mut exits = vec![std::ptr::null_mut(); count as usize];
            shim::lyftar_loop_exit_blocks(self.raw, exits.as_mut_ptr(), count);
            exits.into_iter().map(|exit| BasicBlock::from_raw(exit)).collect()
        }
    }
}

/// How hard a default pipeline optimizes, as an extension point is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptimizationLevel {
    /// `O0`.
    Level0,
    /// `O1`.
    Level1,
    /// `O2`.
    Level2,
    /// `O3`.
    Level3,
}

impl OptimizationLevel {
    fn from_number(number: u32) -> Self {
        match number {
            0 => OptimizationLevel::Level0,
            1 => OptimizationLevel::Level1,
            2 => OptimizationLevel::Level2,
            _ => OptimizationLevel::Level3,
        }
    }
}

/// Where in link-time optimization a pipeline is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkTimeOptimizationPhase {
    /// Not doing link-time optimization.
    None,
    /// Before a thin link.
    ThinPreLink,
    /// After a thin link.
    ThinPostLink,
    /// Before a full link.
    FullPreLink,
    /// After a full link.
    FullPostLink,
}

impl LinkTimeOptimizationPhase {
    fn from_number(number: u32) -> Self {
        match number {
            1 => LinkTimeOptimizationPhase::ThinPreLink,
            2 => LinkTimeOptimizationPhase::ThinPostLink,
            3 => LinkTimeOptimizationPhase::FullPreLink,
            4 => LinkTimeOptimizationPhase::FullPostLink,
            _ => LinkTimeOptimizationPhase::None,
        }
    }
}

// trace: D3
/// The compiler stops at this: a pass claimed it changed nothing, and it did.
#[cold]
#[allow(clippy::panic)]
fn pass_lied(name: &str) -> ! {
    panic!("a pass {name} said it changed nothing, but it changed the IR");
}

// trace: D40, D3
/// The compiler stops at this: a pass left IR that the next C++ pass would crash on.
#[cold]
#[allow(clippy::panic)]
fn pass_left_invalid_ir(name: &str, report: &str) -> ! {
    panic!("a pass {name} left invalid IR: {report}");
}

unsafe extern "C" fn run_module_pass<P: ModulePass>(
    state: *mut c_void,
    module: raw::LLVMModuleRef,
    analyses: *mut c_void,
) -> bool {
    crate::abort_on_panic(|| {
        // SAFETY: the state is the pass this was made with; LLVM gives the module for this call, exclusively.
        let (pass, mut wrapped) = unsafe { (&mut *state.cast::<P>(), Module::from_raw_borrowed(module)) };
        // SAFETY: the module is live.
        let before = unsafe { shim::lyftar_structural_hash_of_module(module) };
        let analyses =
            Analyses { raw: analyses, module, generation: Cell::new(wrapped.generation), marker: PhantomData };
        let result = pass.run(&mut wrapped, &analyses);
        // trace: D40
        if let Err(report) = wrapped.verify() {
            pass_left_invalid_ir(std::any::type_name::<P>(), report.message());
        }
        // SAFETY: the module is live.
        if result == PreservedAnalyses::All && before != unsafe { shim::lyftar_structural_hash_of_module(module) } {
            pass_lied(std::any::type_name::<P>());
        }
        result == PreservedAnalyses::All
    })
}

unsafe extern "C" fn run_function_pass<P: FunctionPass>(
    state: *mut c_void,
    function: raw::LLVMValueRef,
    analyses: *mut c_void,
) -> bool {
    crate::abort_on_panic(|| {
        // SAFETY: the state is the pass this was made with; LLVM gives a function in a module, which is only read here.
        let (pass, module) = unsafe { (&mut *state.cast::<P>(), raw::LLVMGetGlobalParent(function)) };
        // SAFETY: the module and function are live, and the pass only gets what `FunctionInPass` lets it do.
        let mut wrapped = FunctionInPass { module: unsafe { Module::from_raw_borrowed(module) }, function };
        // SAFETY: the function is live.
        let before = unsafe { shim::lyftar_structural_hash_of_function(function) };
        let analyses =
            Analyses { raw: analyses, module, generation: Cell::new(wrapped.module.generation), marker: PhantomData };
        let result = pass.run(&mut wrapped, &analyses);
        // trace: D40
        // A function pass can only change its own function, so that is all this asks about.
        // SAFETY: the function is live, and the shim gives a message for the caller to free, or null.
        let report = unsafe { shim::lyftar_verify_function(function) };
        if !report.is_null() {
            pass_left_invalid_ir(std::any::type_name::<P>(), &from_owned_message(report));
        }
        // SAFETY: the function is live.
        if result == PreservedAnalyses::All && before != unsafe { shim::lyftar_structural_hash_of_function(function) } {
            pass_lied(std::any::type_name::<P>());
        }
        result == PreservedAnalyses::All
    })
}

unsafe extern "C" fn dispose_boxed<T>(state: *mut c_void) {
    // SAFETY: the state came from `Box::into_raw` for this type, and LLVM frees it once.
    drop(unsafe { Box::from_raw(state.cast::<T>()) });
}

/// A module pass manager, as an extension point or a registered pass gets one.
pub struct ModulePassManager<'pipeline> {
    builder: *mut c_void,
    raw: *mut c_void,
    marker: PhantomData<fn(&'pipeline ()) -> &'pipeline ()>,
}

impl<'pipeline> ModulePassManager<'pipeline> {
    /// Adds a module pass.
    pub fn add_module_pass<P: ModulePass + 'pipeline>(&mut self, pass: P) {
        let state = Box::into_raw(Box::new(pass)).cast::<c_void>();
        // SAFETY: the manager is live for this call, and the pass manager owns `state`, freed by the function given.
        unsafe {
            shim::lyftar_module_pass_manager_add_module_pass(self.raw, state, run_module_pass::<P>, dispose_boxed::<P>);
        }
    }

    /// Adds a function pass that runs on each function of the module.
    pub fn add_function_pass<P: FunctionPass + 'pipeline>(&mut self, pass: P) {
        let state = Box::into_raw(Box::new(pass)).cast::<c_void>();
        // SAFETY: the manager is live for this call, and the pass manager owns `state`, freed by the function given.
        unsafe {
            shim::lyftar_module_pass_manager_add_function_pass(
                self.raw,
                state,
                run_function_pass::<P>,
                dispose_boxed::<P>,
            );
        }
    }

    /// Adds passes from LLVM's pipeline text.
    ///
    /// # Errors
    /// What is wrong with the text.
    pub fn add_pipeline(&mut self, pipeline: &str) -> Result<(), Error> {
        check_pipeline_text(pipeline)?;
        // SAFETY: both are live for this call, and LLVM reads `pipeline.len()` bytes.
        Error::from_raw_result(unsafe {
            shim::lyftar_pass_manager_add_pipeline(self.builder, self.raw, 0, pipeline.as_ptr().cast(), pipeline.len())
        })
    }
}

/// A function pass manager, as an extension point or a registered pass gets one.
pub struct FunctionPassManager<'pipeline> {
    builder: *mut c_void,
    raw: *mut c_void,
    marker: PhantomData<fn(&'pipeline ()) -> &'pipeline ()>,
}

impl<'pipeline> FunctionPassManager<'pipeline> {
    /// Adds a function pass.
    pub fn add_function_pass<P: FunctionPass + 'pipeline>(&mut self, pass: P) {
        let state = Box::into_raw(Box::new(pass)).cast::<c_void>();
        // SAFETY: the manager is live for this call, and the pass manager owns `state`, freed by the function given.
        unsafe {
            shim::lyftar_function_pass_manager_add_function_pass(
                self.raw,
                state,
                run_function_pass::<P>,
                dispose_boxed::<P>,
            );
        }
    }

    /// Adds passes from LLVM's pipeline text.
    ///
    /// # Errors
    /// What is wrong with the text.
    pub fn add_pipeline(&mut self, pipeline: &str) -> Result<(), Error> {
        check_pipeline_text(pipeline)?;
        // SAFETY: both are live for this call, and LLVM reads `pipeline.len()` bytes.
        Error::from_raw_result(unsafe {
            shim::lyftar_pass_manager_add_pipeline(self.builder, self.raw, 1, pipeline.as_ptr().cast(), pipeline.len())
        })
    }
}

/// A call-graph pass manager, as the call graph extension point gets one (LLVM's `CGSCCPassManager`).
pub struct CallGraphPassManager<'pipeline> {
    builder: *mut c_void,
    raw: *mut c_void,
    marker: PhantomData<fn(&'pipeline ()) -> &'pipeline ()>,
}

impl<'pipeline> CallGraphPassManager<'pipeline> {
    /// Adds a function pass that runs on each function of each part of the call graph.
    pub fn add_function_pass<P: FunctionPass + 'pipeline>(&mut self, pass: P) {
        let state = Box::into_raw(Box::new(pass)).cast::<c_void>();
        // SAFETY: the manager is live for this call, and the pass manager owns `state`, freed by the function given.
        unsafe {
            shim::lyftar_call_graph_pass_manager_add_function_pass(
                self.raw,
                state,
                run_function_pass::<P>,
                dispose_boxed::<P>,
            );
        }
    }

    /// Adds passes from LLVM's pipeline text.
    ///
    /// # Errors
    /// What is wrong with the text.
    pub fn add_pipeline(&mut self, pipeline: &str) -> Result<(), Error> {
        check_pipeline_text(pipeline)?;
        // SAFETY: both are live for this call, and LLVM reads `pipeline.len()` bytes.
        Error::from_raw_result(unsafe {
            shim::lyftar_pass_manager_add_pipeline(self.builder, self.raw, 2, pipeline.as_ptr().cast(), pipeline.len())
        })
    }
}

/// A loop pass manager, as a loop extension point gets one. It takes LLVM's own loop passes, by pipeline text.
pub struct LoopPassManager<'pipeline> {
    builder: *mut c_void,
    raw: *mut c_void,
    marker: PhantomData<fn(&'pipeline ()) -> &'pipeline ()>,
}

impl LoopPassManager<'_> {
    /// Adds passes from LLVM's pipeline text.
    ///
    /// # Errors
    /// What is wrong with the text.
    pub fn add_pipeline(&mut self, pipeline: &str) -> Result<(), Error> {
        check_pipeline_text(pipeline)?;
        // SAFETY: both are live for this call, and LLVM reads `pipeline.len()` bytes.
        Error::from_raw_result(unsafe {
            shim::lyftar_pass_manager_add_pipeline(self.builder, self.raw, 3, pipeline.as_ptr().cast(), pipeline.len())
        })
    }
}

/// Where a pass can go by name or by extension point, in a pipeline or a plugin.
///
/// What goes on it lives for `'pipeline`.
pub struct PassBuilder<'pipeline> {
    raw: *mut c_void,
    marker: PhantomData<fn(&'pipeline ()) -> &'pipeline ()>,
}

/// Declares a method that adds a callback at one of LLVM's extension points.
macro_rules! extension_point {
    ($(#[$meta:meta])* $name:ident, $number:literal, $manager:ident, simple) => {
        $(#[$meta])*
        pub fn $name<F: FnMut(&mut $manager<'pipeline>, OptimizationLevel) + 'pipeline>(&mut self, callback: F) {
            unsafe extern "C" fn call<'pipeline, F: FnMut(&mut $manager<'pipeline>, OptimizationLevel)>(
                state: *mut c_void,
                builder: *mut c_void,
                manager: *mut c_void,
                level: u32,
                _phase: u32,
            ) {
                crate::abort_on_panic(|| {
                    // SAFETY: the state is the callback this was made with, and LLVM's pass manager lives for this call.
                    let callback = unsafe { &mut *state.cast::<F>() };
                    let mut manager = $manager { builder, raw: manager, marker: PhantomData };
                    callback(&mut manager, OptimizationLevel::from_number(level));
                });
            }
            let state = Box::into_raw(Box::new(callback)).cast::<c_void>();
            // SAFETY: the builder is live for this call, and owns `state`, freed by the function given.
            unsafe {
                shim::lyftar_pass_builder_register_extension_point(
                    self.raw,
                    $number,
                    state,
                    call::<'pipeline, F>,
                    dispose_boxed::<F>,
                );
            }
        }
    };
    ($(#[$meta:meta])* $name:ident, $number:literal, phased) => {
        $(#[$meta])*
        pub fn $name<F: FnMut(&mut ModulePassManager<'pipeline>, OptimizationLevel, LinkTimeOptimizationPhase) + 'pipeline>(
            &mut self,
            callback: F,
        ) {
            unsafe extern "C" fn call<
                'pipeline,
                F: FnMut(&mut ModulePassManager<'pipeline>, OptimizationLevel, LinkTimeOptimizationPhase),
            >(
                state: *mut c_void,
                builder: *mut c_void,
                manager: *mut c_void,
                level: u32,
                phase: u32,
            ) {
                crate::abort_on_panic(|| {
                    // SAFETY: the state is the callback this was made with, and LLVM's pass manager lives for this call.
                    let callback = unsafe { &mut *state.cast::<F>() };
                    let mut manager = ModulePassManager { builder, raw: manager, marker: PhantomData };
                    callback(
                        &mut manager,
                        OptimizationLevel::from_number(level),
                        LinkTimeOptimizationPhase::from_number(phase),
                    );
                });
            }
            let state = Box::into_raw(Box::new(callback)).cast::<c_void>();
            // SAFETY: the builder is live for this call, and owns `state`, freed by the function given.
            unsafe {
                shim::lyftar_pass_builder_register_extension_point(
                    self.raw,
                    $number,
                    state,
                    call::<'pipeline, F>,
                    dispose_boxed::<F>,
                );
            }
        }
    };
}

impl<'pipeline> PassBuilder<'pipeline> {
    /// Wraps LLVM's `PassBuilder`, as a plugin's registration function gets it.
    ///
    /// # Safety
    /// `raw` must be a live `llvm::PassBuilder`, which keeps what is registered for `'pipeline`.
    #[must_use]
    pub unsafe fn from_raw(raw: *mut c_void) -> Self {
        PassBuilder { raw, marker: PhantomData }
    }

    /// Makes a module pass available in pipeline text under `name`. `make` makes one each time the text names it, and LLVM can
    /// call it more than that as it works out what level a name belongs to.
    pub fn register_module_pass<P: ModulePass + 'pipeline, F: FnMut() -> P + 'pipeline>(
        &mut self,
        name: &str,
        make: F,
    ) {
        unsafe extern "C" fn call<'pipeline, P: ModulePass + 'pipeline, F: FnMut() -> P>(
            state: *mut c_void,
            builder: *mut c_void,
            manager: *mut c_void,
        ) {
            crate::abort_on_panic(|| {
                // SAFETY: the state is the closure this was made with, and LLVM's pass manager lives for this call.
                let make = unsafe { &mut *state.cast::<F>() };
                let mut manager: ModulePassManager<'pipeline> =
                    ModulePassManager { builder, raw: manager, marker: PhantomData };
                manager.add_module_pass(make());
            });
        }
        let state = Box::into_raw(Box::new(make)).cast::<c_void>();
        // SAFETY: the builder is live, LLVM copies `name.len()` bytes, and it owns `state`, freed by the function given.
        unsafe {
            shim::lyftar_pass_builder_register_module_pass(
                self.raw,
                name.as_ptr().cast(),
                name.len(),
                state,
                call::<'pipeline, P, F>,
                dispose_boxed::<F>,
            );
        }
    }

    /// Makes a function pass available in pipeline text under `name`. `make` makes one each time the text names it, and LLVM
    /// can call it more than that as it works out what level a name belongs to.
    pub fn register_function_pass<P: FunctionPass + 'pipeline, F: FnMut() -> P + 'pipeline>(
        &mut self,
        name: &str,
        make: F,
    ) {
        unsafe extern "C" fn call<'pipeline, P: FunctionPass + 'pipeline, F: FnMut() -> P>(
            state: *mut c_void,
            builder: *mut c_void,
            manager: *mut c_void,
        ) {
            crate::abort_on_panic(|| {
                // SAFETY: the state is the closure this was made with, and LLVM's pass manager lives for this call.
                let make = unsafe { &mut *state.cast::<F>() };
                let mut manager: FunctionPassManager<'pipeline> =
                    FunctionPassManager { builder, raw: manager, marker: PhantomData };
                manager.add_function_pass(make());
            });
        }
        let state = Box::into_raw(Box::new(make)).cast::<c_void>();
        // SAFETY: the builder is live, LLVM copies `name.len()` bytes, and it owns `state`, freed by the function given.
        unsafe {
            shim::lyftar_pass_builder_register_function_pass(
                self.raw,
                name.as_ptr().cast(),
                name.len(),
                state,
                call::<'pipeline, P, F>,
                dispose_boxed::<F>,
            );
        }
    }

    extension_point!(
        /// After each early clean-up of a function.
        at_peephole, 0, FunctionPassManager, simple
    );
    extension_point!(
        /// After the loop passes that canonicalize loops.
        at_late_loop_optimizations, 1, LoopPassManager, simple
    );
    extension_point!(
        /// At the end of the loop optimizer.
        at_loop_optimizer_end, 2, LoopPassManager, simple
    );
    extension_point!(
        /// At the end of the scalar optimizer.
        at_scalar_optimizer_late, 3, FunctionPassManager, simple
    );
    extension_point!(
        /// At the end of call graph optimization.
        at_call_graph_optimizer_late, 4, CallGraphPassManager, simple
    );
    extension_point!(
        /// Just before the vectorizer.
        at_vectorizer_start, 5, FunctionPassManager, simple
    );
    extension_point!(
        /// Just after the vectorizer.
        at_vectorizer_end, 6, FunctionPassManager, simple
    );
    extension_point!(
        /// At the very start of a default pipeline.
        at_pipeline_start, 7, ModulePassManager, simple
    );
    extension_point!(
        /// After a default pipeline's early simplification.
        at_pipeline_early_simplification, 8, phased
    );
    extension_point!(
        /// At the start of the module optimizer.
        at_optimizer_early, 9, phased
    );
    extension_point!(
        /// At the end of the module optimizer.
        at_optimizer_last, 10, phased
    );
    extension_point!(
        /// At the start of a full link-time optimization pipeline.
        at_full_link_time_optimization_early, 11, ModulePassManager, simple
    );
    extension_point!(
        /// At the end of a full link-time optimization pipeline.
        at_full_link_time_optimization_last, 12, ModulePassManager, simple
    );
}

/// How a pipeline is set up. The fields default to LLVM's own defaults.
// One flag each is how LLVM says these.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassOptions {
    /// Runs the verifier after each pass.
    pub verify_each: bool,
    /// Prints what the pass manager is doing.
    pub debug_logging: bool,
    /// Lets the loop vectorizer interleave.
    pub loop_interleaving: bool,
    /// Runs the loop vectorizer.
    pub loop_vectorization: bool,
    /// Runs the vectorizer that joins separate operations (LLVM's SLP vectorizer).
    pub superword_level_parallelism_vectorization: bool,
    /// Unrolls loops.
    pub loop_unrolling: bool,
    /// Has loop unrolling forget all it knows about loops' scalar evolution.
    pub forget_all_scalar_evolution_in_loop_unroll: bool,
    /// How many clobber queries loop-invariant code motion makes of memory SSA, at most.
    pub licm_memory_ssa_optimization_cap: u32,
    /// How many accesses loop-invariant code motion will look at for promotion, at most.
    pub licm_memory_ssa_no_access_for_promotion_cap: u32,
    /// Adds the call graph profile pass.
    pub call_graph_profile: bool,
    /// Adds the pass that merges identical functions.
    pub merge_functions: bool,
    /// The inliner's threshold, or `None` for the level's own.
    pub inliner_threshold: Option<i32>,
    /// LLVM's text for the alias analysis pipeline, if not the default.
    pub alias_analysis_pipeline: Option<String>,
}

impl Default for PassOptions {
    fn default() -> Self {
        // LLVM's `PipelineTuningOptions` defaults, as `PassBuilder` starts from them.
        PassOptions {
            verify_each: false,
            debug_logging: false,
            loop_interleaving: true,
            loop_vectorization: true,
            superword_level_parallelism_vectorization: true,
            loop_unrolling: true,
            forget_all_scalar_evolution_in_loop_unroll: false,
            licm_memory_ssa_optimization_cap: 100,
            licm_memory_ssa_no_access_for_promotion_cap: 250,
            call_graph_profile: true,
            merge_functions: false,
            inliner_threshold: None,
            alias_analysis_pipeline: None,
        }
    }
}

/// Owns LLVM's C form of [`PassOptions`] for a call, and the alias analysis text, which LLVM keeps without copying.
struct RawPassOptions(raw::LLVMPassBuilderOptionsRef, Option<CString>);

impl Drop for RawPassOptions {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the options.
        unsafe { raw::LLVMDisposePassBuilderOptions(self.0) };
    }
}

impl PassOptions {
    fn to_raw(&self) -> Result<RawPassOptions, Error> {
        // SAFETY: takes nothing, and the options are owned by the wrapper from here on.
        let mut options = RawPassOptions(unsafe { raw::LLVMCreatePassBuilderOptions() }, None);
        // SAFETY: the options are live, and the one text LLVM keeps is owned by the wrapper, so it outlives them.
        unsafe {
            raw::LLVMPassBuilderOptionsSetVerifyEach(options.0, i32::from(self.verify_each));
            raw::LLVMPassBuilderOptionsSetDebugLogging(options.0, i32::from(self.debug_logging));
            raw::LLVMPassBuilderOptionsSetLoopInterleaving(options.0, i32::from(self.loop_interleaving));
            raw::LLVMPassBuilderOptionsSetLoopVectorization(options.0, i32::from(self.loop_vectorization));
            raw::LLVMPassBuilderOptionsSetSLPVectorization(
                options.0,
                i32::from(self.superword_level_parallelism_vectorization),
            );
            raw::LLVMPassBuilderOptionsSetLoopUnrolling(options.0, i32::from(self.loop_unrolling));
            raw::LLVMPassBuilderOptionsSetForgetAllSCEVInLoopUnroll(
                options.0,
                i32::from(self.forget_all_scalar_evolution_in_loop_unroll),
            );
            raw::LLVMPassBuilderOptionsSetLicmMssaOptCap(options.0, self.licm_memory_ssa_optimization_cap);
            raw::LLVMPassBuilderOptionsSetLicmMssaNoAccForPromotionCap(
                options.0,
                self.licm_memory_ssa_no_access_for_promotion_cap,
            );
            raw::LLVMPassBuilderOptionsSetCallGraphProfile(options.0, i32::from(self.call_graph_profile));
            raw::LLVMPassBuilderOptionsSetMergeFunctions(options.0, i32::from(self.merge_functions));
            if let Some(threshold) = self.inliner_threshold {
                raw::LLVMPassBuilderOptionsSetInlinerThreshold(options.0, threshold);
            }
            if let Some(pipeline) = &self.alias_analysis_pipeline {
                let pipeline = CString::new(pipeline.as_str())
                    .map_err(|_| Error::new("an alias analysis pipeline has no NUL bytes in it"))?;
                raw::LLVMPassBuilderOptionsSetAAPipeline(options.0, pipeline.as_ptr());
                options.1 = Some(pipeline);
            }
        }
        Ok(options)
    }
}

// trace: D3, D38
/// Finds in pipeline text what LLVM would end the process over, or crash on, rather than give an error, so it can be one.
///
/// This can only know what has been seen against the real library. Those were found by running every pass `opt` names.
fn check_pipeline_text(pipeline: &str) -> Result<(), Error> {
    const OPTIMIZING_PIPELINES: [&str; 5] = ["default", "thinlto-pre-link", "thinlto", "lto-pre-link", "lto"];
    let is_name_character = |character: char| character.is_alphanumeric() || matches!(character, '-' | '_' | '.');
    for name in pipeline.split(|character: char| !is_name_character(character)) {
        // SAFETY: takes nothing, and only reads LLVM's list of options.
        if name == "function-import" && !unsafe { shim::lyftar_summary_file_was_given() } {
            return Err(Error::new("the function-import pass needs LLVM's -summary-file option to be given"));
        }
        if name == "free-machine-function" {
            return Err(Error::new(
                "the free-machine-function pass needs a machine function analysis, which this pipeline doesn't have",
            ));
        }
    }
    for (level, attribute) in [("Os", "optsize"), ("Oz", "minsize")] {
        for (start, _) in pipeline.match_indices(&format!("<{level}>")) {
            let before = &pipeline[..start];
            let name_start = before.rfind(|character: char| !is_name_character(character)).map_or(0, |found| found + 1);
            if OPTIMIZING_PIPELINES.contains(&&before[name_start..]) {
                return Err(Error::new(format!(
                    "the optimization level \"{level}\" is no longer supported: use O2 with the {attribute} attribute"
                )));
            }
        }
    }
    Ok(())
}

impl Module<'_> {
    // trace: D45
    /// `place-safepoints` inlines a `gc.safepoint_poll` the module has to define, and without one LLVM reads a null function.
    fn check_safepoint_poll(&self, pipeline: &str) -> Result<(), Error> {
        // SAFETY: the module is live, and the shim only reads it.
        if pipeline.contains("place-safepoints") && unsafe { shim::lyftar_module_needs_safepoint_poll(self.raw) } {
            return Err(Error::new(
                "the place-safepoints pass needs this module to define a function `void gc.safepoint_poll()`, which it inlines",
            ));
        }
        Ok(())
    }
}

fn raw_machine(machine: Option<&TargetMachine>) -> raw::LLVMTargetMachineRef {
    machine.map_or(std::ptr::null_mut(), |found| found.raw)
}

impl Module<'_> {
    // trace: D11
    /// Runs LLVM's own passes over this module from pipeline text, such as `default<O3>`.
    ///
    /// Passes can delete anything, so this needs exclusive access: a value held across it doesn't compile.
    ///
    /// # Errors
    /// What is wrong with the text, or the verifier's report if the module is invalid, since LLVM would crash on it.
    ///
    /// # Panics
    /// If `pipeline` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn run_passes(
        &mut self,
        pipeline: &str,
        machine: Option<&TargetMachine>,
        options: &PassOptions,
    ) -> Result<(), Error> {
        check_pipeline_text(pipeline)?;
        self.verify_before("run_passes")?;
        self.check_safepoint_poll(pipeline)?;
        let pipeline = c_string(pipeline, "run_passes");
        let raw_options = options.to_raw()?;
        self.bump_generation();
        // SAFETY: the module is held exclusively, the text is a valid C string for the call, a null machine is allowed,
        // and LLVM only reads the options.
        Error::from_raw_result(unsafe {
            raw::LLVMRunPasses(self.raw, pipeline.as_ptr(), raw_machine(machine), raw_options.0)
        })
    }

    // trace: D11
    /// Runs LLVM's own function passes over just the function called `function_name`, from pipeline text.
    ///
    /// LLVM may delete instructions there, so this needs exclusive access: a value held across it doesn't compile.
    ///
    /// # Errors
    /// What is wrong with the text, or the verifier's report if the module is invalid, since LLVM would crash on it.
    ///
    /// # Panics
    /// If this module has no such function, or the text has a NUL byte inside it (D3).
    #[track_caller]
    pub fn run_passes_on_function(
        &mut self,
        function_name: &str,
        pipeline: &str,
        machine: Option<&TargetMachine>,
        options: &PassOptions,
    ) -> Result<(), Error> {
        check_pipeline_text(pipeline)?;
        self.verify_before("run_passes_on_function")?;
        self.check_safepoint_poll(pipeline)?;
        let pipeline = c_string(pipeline, "run_passes_on_function");
        let raw_options = options.to_raw()?;
        let Some(function) = self.get_function(function_name) else {
            bad_call(format_args!("run_passes_on_function: this module has no function {function_name}"))
        };
        let function = function.raw;
        self.bump_generation();
        // SAFETY: the module is held exclusively, the text is a valid C string for the call, a null machine is allowed,
        // and LLVM only reads the options.
        Error::from_raw_result(unsafe {
            raw::LLVMRunPassesOnFunction(function, pipeline.as_ptr(), raw_machine(machine), raw_options.0)
        })
    }

    // trace: D11
    /// Runs a pipeline with Rust passes in it. `register` gets the [`PassBuilder`] before the text is read, to name Rust's
    /// passes and put them at LLVM's extension points.
    ///
    /// Passes can delete anything, so this needs exclusive access: a value held across it doesn't compile.
    ///
    /// # Errors
    /// What is wrong with the text, or the verifier's report if the module is invalid, since LLVM would crash on it.
    pub fn run_passes_with<'pipeline>(
        &mut self,
        pipeline: &str,
        machine: Option<&TargetMachine>,
        options: &PassOptions,
        register: impl FnOnce(&mut PassBuilder<'pipeline>) + 'pipeline,
    ) -> Result<(), Error> {
        unsafe extern "C" fn call<'pipeline, F: FnOnce(&mut PassBuilder<'pipeline>)>(
            state: *mut c_void,
            builder: *mut c_void,
        ) {
            crate::abort_on_panic(|| {
                // SAFETY: the state is the `Option` below, which is live for the call, and the builder lasts as long as
                // everything registered on it.
                let register = unsafe { &mut *state.cast::<Option<F>>() };
                let mut builder = PassBuilder { raw: builder, marker: PhantomData };
                if let Some(register) = register.take() {
                    register(&mut builder);
                }
            });
        }
        fn pointer_to<'pipeline, F: FnOnce(&mut PassBuilder<'pipeline>)>(
            register: &mut Option<F>,
        ) -> (*mut c_void, shim::Register) {
            (std::ptr::from_mut(register).cast(), call::<'pipeline, F>)
        }
        check_pipeline_text(pipeline)?;
        self.verify_before("run_passes_with")?;
        self.bump_generation();
        let (alias_pipeline, alias_length) = options
            .alias_analysis_pipeline
            .as_ref()
            .map_or((std::ptr::null::<c_char>(), 0), |text| (text.as_ptr().cast(), text.len()));
        let shim_options = shim::PassOptionsForShim {
            verify_each: options.verify_each,
            debug_logging: options.debug_logging,
            loop_interleaving: options.loop_interleaving,
            loop_vectorization: options.loop_vectorization,
            superword_level_parallelism_vectorization: options.superword_level_parallelism_vectorization,
            loop_unrolling: options.loop_unrolling,
            forget_all_scalar_evolution_in_loop_unroll: options.forget_all_scalar_evolution_in_loop_unroll,
            call_graph_profile: options.call_graph_profile,
            merge_functions: options.merge_functions,
            licm_memory_ssa_optimization_cap: options.licm_memory_ssa_optimization_cap,
            licm_memory_ssa_no_access_for_promotion_cap: options.licm_memory_ssa_no_access_for_promotion_cap,
            inliner_threshold: options.inliner_threshold.unwrap_or(-1),
            alias_analysis_pipeline: alias_pipeline,
            alias_analysis_pipeline_length: alias_length,
        };
        let mut register = Some(register);
        let (state, register_call) = pointer_to(&mut register);
        // SAFETY: the module is held exclusively; LLVM reads `pipeline.len()` bytes and the options; a null machine is
        // allowed; the state and the options last for this call, and everything on the builder goes when it returns.
        Error::from_raw_result(unsafe {
            shim::lyftar_run_passes_with(
                self.raw,
                pipeline.as_ptr().cast(),
                pipeline.len(),
                raw_machine(machine),
                &raw const shim_options,
                state,
                register_call,
            )
        })
    }
}

// trace: D1
/// Makes this crate a pass plugin for `opt -load-pass-plugin` or `clang -fpass-plugin`: the crate is a `cdylib`, and this
/// names it and says what it registers, as `lyftar::pass_plugin!("name", "version", register)` where `register` takes a
/// `&mut PassBuilder<'static>`.
///
/// Not on Windows, where it is a compile error (D44), nor with Lyftar's static LLVM, where it is one too (D53).
#[cfg(not(any(target_os = "windows", lyftar_static_llvm)))]
#[macro_export]
macro_rules! pass_plugin {
    ($name:literal, $version:literal, $register:path) => {
        /// LLVM looks this up by name when it loads a plugin.
        #[unsafe(no_mangle)]
        pub extern "C" fn llvmGetPassPluginInfo() -> $crate::shim::PluginInformation {
            extern "C" fn register_callbacks(builder: *mut ::std::ffi::c_void) {
                // SAFETY: LLVM gives a live `PassBuilder` here, and keeps a plugin's callbacks for good.
                unsafe { $crate::passes::run_plugin_registration(builder, $register) };
            }
            $crate::shim::PluginInformation {
                // LLVM_PLUGIN_API_VERSION in llvm/Plugins/PassPlugin.h.
                api_version: 2,
                plugin_name: concat!($name, "\0").as_ptr().cast(),
                plugin_version: concat!($version, "\0").as_ptr().cast(),
                register_callbacks,
                pre_code_generation_callback: ::std::ptr::null(),
            }
        }
    };
}

// trace: D44
/// Windows' official LLVM is built with plugins off, so a plugin would carry its own copy of LLVM, separate from `opt`'s.
#[cfg(target_os = "windows")]
#[macro_export]
macro_rules! pass_plugin {
    ($($anything:tt)*) => {
        compile_error!(
            "Lyftar's plugins don't work on Windows yet, because the official LLVM there is built with plugins off. \
             Passes linked into a compiler work, through `Module::run_passes_with`."
        );
    };
}

// trace: D53
/// A plugin built with a static LLVM, such as the one `build.rs` downloads, would carry its own copy of LLVM into a program that has one.
#[cfg(all(lyftar_static_llvm, not(target_os = "windows")))]
#[macro_export]
macro_rules! pass_plugin {
    ($($anything:tt)*) => {
        compile_error!(
            "Lyftar's plugins need a shared LLVM: point LYFTAR_LLVM_CONFIG at one, such as Homebrew's. With the static LLVM a plugin \
             would carry its own copy into a program that has one. Passes linked into a compiler work, through `Module::run_passes_with`."
        );
    };
}

/// What [`pass_plugin!`](crate::pass_plugin) calls. A panic in it aborts `opt` or `clang`, since it is running inside LLVM.
///
/// # Safety
/// `builder` must be a live `llvm::PassBuilder`, which keeps what is registered for good.
#[doc(hidden)]
pub unsafe fn run_plugin_registration(builder: *mut c_void, register: fn(&mut PassBuilder<'static>)) {
    crate::abort_on_panic(|| {
        // SAFETY: the caller promises a live `PassBuilder` that keeps what is registered for good.
        let mut builder: PassBuilder<'static> = unsafe { PassBuilder::from_raw(builder) };
        register(&mut builder);
    });
}
