//! kayo's recursive inliner (CyborgPL/kayo/llvm-passes/recursive_inline.cpp), in Rust.
//!
//! A function that calls itself is inlined into itself a few levels deep, as GCC's `max-inline-recursive-depth` does, which
//! none of LLVM's inliners will. The body is copied once, as it is, into a snapshot; each call the function makes to itself
//! is pointed at the snapshot and inlined, which leaves calls to the function itself one level further down; repeated to a
//! depth, and stopped where the function would grow past a size. The snapshot is then dropped. Two functions calling each
//! other are first joined: each partner is inlined where the other calls it, which leaves each calling itself.

use std::cell::Cell;

use lyftar::Value;
use lyftar::attributes::{AttributeKind, AttributeLocation};
use lyftar::enums::Linkage;
use lyftar::module::{Module, TrackedHandle};
use lyftar::passes::{Analyses, ModulePass, PreservedAnalyses};
use lyftar::values::{CallBase, Function};

/// Recursive inlining, as a module pass.
pub struct RecursiveInline<'report> {
    /// How many levels a function is inlined into itself, at most.
    pub depth: u32,
    /// How many instructions a function may grow to, at most.
    pub max_size: u32,
    /// Counts the calls inlined, for the report.
    pub inlined: Option<&'report Cell<u32>>,
}

/// The calls `function` makes to itself, directly.
fn self_calls<'context, 'module>(
    function: Value<'context, 'module, Function>,
) -> Vec<Value<'context, 'module, CallBase>> {
    function
        .function_instructions()
        .filter_map(|instruction| instruction.as_kind::<CallBase>())
        .filter(|call| call.called_function() == Some(function))
        .collect()
}

fn has_attribute(function: Value<'_, '_, Function>, name: &str) -> bool {
    AttributeKind::for_name(name)
        .is_some_and(|kind| function.enum_attribute(AttributeLocation::Function, kind).is_some())
}

/// Whether `function` may be inlined anywhere: a body, nothing that asks it to stay whole.
fn inlinable(function: Value<'_, '_, Function>) -> bool {
    !function.is_declaration()
        && !has_attribute(function, "noinline")
        && !has_attribute(function, "optnone")
        && !function.value_type().is_variadic()
}

/// Whether `function` may be inlined into itself: inlinable, and calling itself.
fn may_unfold(function: Value<'_, '_, Function>) -> bool {
    inlinable(function) && !self_calls(function).is_empty()
}

/// Whether `function` calls `other` directly.
fn calls<'context, 'module>(
    function: Value<'context, 'module, Function>,
    other: Value<'context, 'module, Function>,
) -> bool {
    function
        .function_instructions()
        .filter_map(|instruction| instruction.as_kind::<CallBase>())
        .any(|call| call.called_function() == Some(other))
}

/// Inlines `call`, pointed at `snapshot` for the time being, and puts it back if LLVM refuses. Gives back whether it inlined.
fn inline_through_snapshot<'context>(
    module: &mut Module<'context>,
    call: &TrackedHandle<'context, CallBase>,
    snapshot: &TrackedHandle<'context, Function>,
    original: &TrackedHandle<'context, Function>,
) -> bool {
    let choice = {
        let (Some(found), Some(snapshot), true) = (call.get(module), snapshot.get(module), original.is_present())
        else {
            return false;
        };
        found.set_called_function(snapshot);
        module.call_to_inline(found)
    };
    if module.inline_call(choice).is_ok() {
        return true;
    }
    if let (Some(found), Some(original)) = (call.get(module), original.get(module)) {
        found.set_called_function(original);
    }
    false
}

/// Two functions calling each other: where `f` calls a `g` that calls `f` back, `g`'s body, as it was before anything here,
/// is inlined into `f`, so `f` calls itself and is unfolded as any function that does. Gives back how many calls were
/// inlined.
fn join_pairs<'context>(module: &mut Module<'context>, max_size: u32) -> u32 {
    struct Pair<'context> {
        caller: TrackedHandle<'context, Function>,
        call: TrackedHandle<'context, CallBase>,
        partner: TrackedHandle<'context, Function>,
    }
    let mut pairs = Vec::new();
    for caller in module.functions() {
        if !inlinable(caller) {
            continue;
        }
        for instruction in caller.function_instructions() {
            let Some(call) = instruction.as_kind::<CallBase>() else {
                continue;
            };
            let Some(partner) = call.called_function() else {
                continue;
            };
            if partner != caller
                && inlinable(partner)
                && partner.instruction_count() <= max_size
                && calls(partner, caller)
            {
                pairs.push(Pair {
                    caller: module.track(caller),
                    call: module.track(call),
                    partner: module.track(partner),
                });
            }
        }
    }
    // Every partner's body as it was, before any of them changes.
    let mut snapshots: Vec<(TrackedHandle<'context, Function>, TrackedHandle<'context, Function>)> = Vec::new();
    for pair in &pairs {
        let Some(partner) = pair.partner.get(module) else {
            continue;
        };
        let already = snapshots.iter().any(|(original, _)| original.get(module) == Some(partner));
        if !already {
            let snapshot = partner.clone_function();
            snapshot.set_linkage(Linkage::Internal);
            snapshots.push((module.track(partner), module.track(snapshot)));
        }
    }
    let mut inlined = 0;
    for pair in &pairs {
        let (Some(caller), Some(partner)) = (pair.caller.get(module), pair.partner.get(module)) else {
            continue;
        };
        if caller.instruction_count() + partner.instruction_count() > max_size {
            continue;
        }
        let Some((_, snapshot)) = snapshots.iter().find(|(original, _)| original.get(module) == Some(partner)) else {
            continue;
        };
        if inline_through_snapshot(module, &pair.call, snapshot, &pair.partner) {
            inlined += 1;
        }
    }
    let to_erase: Vec<_> = snapshots
        .iter()
        .filter_map(|(_, snapshot)| snapshot.get(module))
        .map(|snapshot| module.value_to_erase(snapshot))
        .collect();
    module.erase(&to_erase);
    inlined
}

/// Inlines `function` into itself up to `depth` levels, while it stays within `max_size` instructions. Gives back how many
/// calls were inlined.
fn unfold<'context>(
    module: &mut Module<'context>,
    function: &TrackedHandle<'context, Function>,
    depth: u32,
    max_size: u32,
) -> u32 {
    let (snapshot, body_size) = {
        let Some(found) = function.get(module) else {
            return 0;
        };
        let snapshot = found.clone_function();
        snapshot.set_linkage(Linkage::Internal);
        (module.track(snapshot), snapshot.instruction_count())
    };
    let mut inlined = 0;
    for _level in 0..depth {
        let calls: Vec<TrackedHandle<'context, CallBase>> = {
            let Some(found) = function.get(module) else {
                break;
            };
            let calls = self_calls(found);
            let count = u32::try_from(calls.len()).unwrap_or(u32::MAX);
            if calls.is_empty() || found.instruction_count().saturating_add(count.saturating_mul(body_size)) > max_size
            {
                break;
            }
            calls.into_iter().map(|call| module.track(call)).collect()
        };
        for call in &calls {
            if inline_through_snapshot(module, call, &snapshot, function) {
                inlined += 1;
            }
        }
    }
    if let Some(found) = snapshot.get(module) {
        let choice = module.value_to_erase(found);
        module.erase(&[choice]);
    }
    inlined
}

impl ModulePass for RecursiveInline<'_> {
    fn run<'context>(&mut self, module: &mut Module<'context>, _analyses: &Analyses<'_>) -> PreservedAnalyses {
        let joined = join_pairs(module, self.max_size);
        let recursive: Vec<TrackedHandle<'context, Function>> = module
            .functions()
            .filter(|function| may_unfold(*function))
            .map(|function| module.track(function))
            .collect();
        let mut count = joined;
        for function in &recursive {
            count += unfold(module, function, self.depth, self.max_size);
        }
        if let Some(report) = self.inlined {
            report.set(report.get() + count);
        }
        if count == 0 { PreservedAnalyses::All } else { PreservedAnalyses::None }
    }
}
