//! A small two-space copying collector for what Lyftar compiles (D45). The test's program runs one at a time, on one thread.
//!
//! An object is a header word holding its pointer and data field counts, a forwarding word, then its fields. A collection copies
//! every object it can reach to a new space, rewrites each root and poisons the old space, which it keeps, so a root it missed
//! reads poison and can't land in a later space. It then finds each root again to prove they are all in the new space, and
//! allocating always collects.

use std::cell::UnsafeCell;
use std::fmt::Write;

use lyftar_stack_maps::{Location, Root, StackMaps, loaded_sections};

const SPACE_SIZE_IN_WORDS: usize = 1 << 13;
const HEADER_SIZE_IN_WORDS: usize = 2;
const POISON: u64 = 0xDEAD_DEAD_DEAD_DEAD;

/// LLVM's DWARF numbers for the stack pointer and frame pointer.
#[cfg(target_arch = "aarch64")]
const REGISTERS: (u16, u16) = (31, 29);
#[cfg(target_arch = "x86_64")]
const REGISTERS: (u16, u16) = (7, 6);

/// How far the stack pointer moves when a call returns, past a function's stack size.
#[cfg(target_arch = "aarch64")]
const RETURN_ADDRESS_SIZE_IN_BYTES: u64 = 0;
#[cfg(target_arch = "x86_64")]
const RETURN_ADDRESS_SIZE_IN_BYTES: u64 = 8;

/// How the shadow stack's lowering lays out a frame: its map says how many root slots follow.
#[repr(C)]
pub struct FrameMap {
    root_count: i32,
    metadata_count: i32,
}

/// One frame on the shadow stack.
#[repr(C)]
pub struct StackEntry {
    next: *mut StackEntry,
    map: *const FrameMap,
    roots: [*mut u64; 0],
}

enum RootFinder {
    Unset,
    StackMaps(StackMaps<'static>),
    ShadowStack(*mut *mut StackEntry),
    FrameTable(Vec<FrameDescriptor>),
}

/// One entry of the `ocaml` strategy's frame table.
struct FrameDescriptor {
    return_address: u64,
    frame_size_in_bytes: u16,
    /// Where its live offsets are.
    offsets: *const u16,
    offset_count: u16,
}

struct State {
    space: Vec<u64>,
    free_in_words: usize,
    /// Every space collected from, poisoned.
    old_spaces: Vec<Vec<u64>>,
    /// Where a collection is copying to.
    next_space: Vec<u64>,
    next_free_in_words: usize,
    finder: RootFinder,
    preset_sections: Vec<Vec<u8>>,
    output: String,
    collections: u64,
    objects_moved: u64,
    roots_rewritten: u64,
    derived_pointers_rewritten: u64,
    frames_walked: u64,
}

struct Global(UnsafeCell<Option<State>>);
// SAFETY: one program runs at a time, on one thread, and the test holds a lock across it.
unsafe impl Sync for Global {}
static STATE: Global = Global(UnsafeCell::new(None));

fn state() -> &'static mut State {
    // SAFETY: as for `Global`. Nothing holds this across a call that gets it again.
    let slot = unsafe { &mut *STATE.0.get() };
    slot.get_or_insert_with(|| State {
        space: vec![0; SPACE_SIZE_IN_WORDS],
        free_in_words: 0,
        old_spaces: Vec::new(),
        next_space: Vec::new(),
        next_free_in_words: 0,
        finder: RootFinder::Unset,
        preset_sections: Vec::new(),
        output: String::new(),
        collections: 0,
        objects_moved: 0,
        roots_rewritten: 0,
        derived_pointers_rewritten: 0,
        frames_walked: 0,
    })
}

/// Starts again, with these stack map sections from a JIT if it has them.
pub fn reset(sections: Vec<Vec<u8>>) {
    // SAFETY: as for `Global`.
    unsafe { *STATE.0.get() = None };
    state().preset_sections = sections;
}

/// What the program reported.
pub fn take_output() -> String {
    std::mem::take(&mut state().output)
}

fn inside(space: &[u64], address: u64) -> bool {
    let start = space.as_ptr() as u64;
    address >= start && address < start + (space.len() * 8) as u64
}

/// Copies this object to the next space if it isn't there yet, and says where it is now.
fn forward(address: u64) -> u64 {
    let state = state();
    if address == 0 || inside(&state.next_space, address) {
        return address;
    }
    assert!(inside(&state.space, address), "a root that isn't in the heap: {address:#x}");
    let object = address as *mut u64;
    // SAFETY: a root in the heap points at an object's header, which stays there until this collection frees the space.
    unsafe {
        let forwarded = *object.add(1);
        if forwarded != 0 {
            return forwarded;
        }
        let header = *object;
        let size_in_words = HEADER_SIZE_IN_WORDS + (header >> 32) as usize + (header & 0xffff_ffff) as usize;
        assert!(state.next_free_in_words + size_in_words <= SPACE_SIZE_IN_WORDS, "out of memory");
        let copy = state.next_space.as_mut_ptr().add(state.next_free_in_words);
        std::ptr::copy_nonoverlapping(object.cast_const(), copy, size_in_words);
        *copy.add(1) = 0;
        *object.add(1) = copy as u64;
        state.next_free_in_words += size_in_words;
        state.objects_moved += 1;
        copy as u64
    }
}

fn slot_of(location: Location, stack_pointer: u64, frame_pointer: u64) -> Option<*mut u64> {
    let Location::Indirect { register, offset_in_bytes, .. } = location else {
        return None;
    };
    let base = if register == REGISTERS.0 {
        stack_pointer
    } else if register == REGISTERS.1 {
        frame_pointer
    } else {
        panic!("a live pointer based on register {register}, which this collector doesn't know")
    };
    Some(base.wrapping_add_signed(i64::from(offset_in_bytes)) as *mut u64)
}

fn allocation_slot(location: Location, stack_pointer: u64, frame_pointer: u64) -> Option<*mut u64> {
    let Location::Direct { register, offset_in_bytes, size_in_bytes } = location else {
        return None;
    };
    assert_eq!(size_in_bytes, 8, "the test's stack allocations hold one pointer");
    slot_of(Location::Indirect { register, offset_in_bytes, size_in_bytes }, stack_pointer, frame_pointer)
}

/// What to do with each pair of a frame's roots: rewrite them, or check they have been.
#[derive(Clone, Copy, PartialEq)]
enum Visit {
    Rewrite,
    Check,
}

/// Visits a frame's roots in three steps, which is what lets one slot be in several pairs: each derived pointer becomes an offset
/// while every base is still old, each base moves, and each derived pointer is rebuilt from it. This allocates nothing.
fn visit_statepoint_frame(
    stack_maps: &StackMaps<'static>,
    return_address: u64,
    stack_pointer: u64,
    frame_pointer: u64,
    visit: Visit,
) -> bool {
    let Some(call) = stack_maps.find(return_address) else {
        return false;
    };
    let statepoint = call.record.statepoint().expect("a statepoint's record");
    if call.function.stack_size_in_bytes != u64::MAX {
        assert_eq!(
            stack_pointer + call.function.stack_size_in_bytes + RETURN_ADDRESS_SIZE_IN_BYTES,
            frame_pointer + 16,
            "the stack map's frame size"
        );
    }
    let slots = |root: Root| match root {
        Root::Pair { base, derived } => {
            (slot_of(base, stack_pointer, frame_pointer), slot_of(derived, stack_pointer, frame_pointer))
        }
        // An allocation holding one pointer is a slot that is its own base.
        Root::Allocation(location) => {
            let slot = allocation_slot(location, stack_pointer, frame_pointer);
            (slot, slot)
        }
    };
    if visit == Visit::Check {
        let current = &state().space;
        for root in statepoint.roots() {
            for slot in [slots(root).0, slots(root).1].into_iter().flatten() {
                // SAFETY: the stack map says this slot is in this frame, which is suspended in this call.
                let value = unsafe { *slot };
                assert!(value == 0 || inside(current, value), "a root wasn't rewritten: {value:#x}");
            }
        }
        return true;
    }
    // A pair listed twice, or a slot already in a derived step, must only be done once.
    let seen_before = |index: usize, wanted: (Option<*mut u64>, Option<*mut u64>)| {
        statepoint.roots().take(index).any(|earlier| {
            let earlier = slots(earlier);
            earlier == wanted || (earlier.1 == wanted.1 && wanted.0 != wanted.1)
        })
    };
    // SAFETY: as above.
    unsafe {
        for (index, root) in statepoint.roots().enumerate() {
            let (base, derived) = slots(root);
            if let (Some(base), Some(derived)) = (base, derived)
                && base != derived
                && !seen_before(index, (Some(base), Some(derived)))
            {
                *derived = (*derived).wrapping_sub(*base);
            }
        }
        for root in statepoint.roots() {
            if let Some(base) = slots(root).0 {
                let moved = forward(*base);
                if moved != *base {
                    state().roots_rewritten += 1;
                }
                *base = moved;
            }
        }
        for (index, root) in statepoint.roots().enumerate() {
            let (base, derived) = slots(root);
            if let (Some(base), Some(derived)) = (base, derived)
                && base != derived
                && !seen_before(index, (Some(base), Some(derived)))
            {
                *derived = (*derived).wrapping_add(*base);
                state().derived_pointers_rewritten += 1;
            }
        }
    }
    true
}

fn visit_table_frame(
    table: &[FrameDescriptor],
    return_address: u64,
    stack_pointer: u64,
    frame_pointer: u64,
    visit: Visit,
) -> bool {
    let Ok(found) = table.binary_search_by_key(&return_address, |descriptor| descriptor.return_address) else {
        return false;
    };
    let descriptor = &table[found];
    for index in 0..usize::from(descriptor.offset_count) {
        // SAFETY: the frame table says each of these is a stack offset in this frame, which is suspended in this call. AArch64
        // gives it from the frame pointer, as a negative number, and otherwise it is from the stack pointer.
        unsafe {
            let offset = (*descriptor.offsets.add(index)).cast_signed();
            let base = if offset < 0 { frame_pointer } else { stack_pointer };
            let slot = base.wrapping_add_signed(i64::from(offset)) as *mut u64;
            if visit == Visit::Check {
                assert!(*slot == 0 || inside(&state().space, *slot), "a root wasn't rewritten: {:#x}", *slot);
            } else {
                let moved = forward(*slot);
                if moved != *slot {
                    state().roots_rewritten += 1;
                }
                *slot = moved;
            }
        }
    }
    let _ = descriptor.frame_size_in_bytes;
    true
}

fn visit_shadow_stack(head: *mut *mut StackEntry, visit: Visit) {
    // SAFETY: the program's lowering keeps this chain of live frames, each with as many root slots as its map says.
    unsafe {
        let mut entry = *head;
        while !entry.is_null() {
            state().frames_walked += 1;
            let roots = (&raw mut (*entry).roots).cast::<*mut u64>();
            for index in 0..(*(*entry).map).root_count as usize {
                let slot = roots.add(index);
                let value = *slot as u64;
                if visit == Visit::Check {
                    assert!(value == 0 || inside(&state().space, value), "a root wasn't rewritten: {value:#x}");
                } else {
                    let moved = forward(value);
                    if moved != value {
                        state().roots_rewritten += 1;
                    }
                    *slot = moved as *mut u64;
                }
            }
            entry = (*entry).next;
        }
    }
}

/// Walks the program's frames from the call into the collector, by frame pointer, until a return address the tables don't know.
fn walk_frames(mut return_address: u64, mut stack_pointer: u64, mut frame_pointer: u64, visit: Visit) {
    let finder: *const RootFinder = &state().finder;
    // SAFETY: nothing changes the finder during a collection, and these frames are all suspended in calls. Each one keeps a frame
    // pointer, so its record has the caller's frame pointer and this frame's return address, and its caller's stack starts after.
    unsafe {
        if let RootFinder::ShadowStack(head) = &*finder {
            visit_shadow_stack(*head, visit);
            return;
        }
        loop {
            let found = match &*finder {
                RootFinder::StackMaps(stack_maps) => {
                    visit_statepoint_frame(stack_maps, return_address, stack_pointer, frame_pointer, visit)
                }
                RootFinder::FrameTable(table) => {
                    visit_table_frame(table, return_address, stack_pointer, frame_pointer, visit)
                }
                RootFinder::Unset | RootFinder::ShadowStack(_) => panic!("the program said nothing about its roots"),
            };
            if !found {
                return;
            }
            state().frames_walked += 1;
            let record = frame_pointer as *const u64;
            stack_pointer = frame_pointer + 16;
            return_address = *record.add(1);
            frame_pointer = *record;
        }
    }
}

fn collect(return_address: u64, stack_pointer: u64, frame_pointer: u64) {
    let current = state();
    current.next_space = vec![0; SPACE_SIZE_IN_WORDS];
    current.next_free_in_words = 0;
    current.collections += 1;
    walk_frames(return_address, stack_pointer, frame_pointer, Visit::Rewrite);
    // Every object the roots reach, field by field, as they are copied.
    let current = state();
    let mut scan = 0;
    while scan < current.next_free_in_words {
        let header = current.next_space[scan];
        let pointer_count = (header >> 32) as usize;
        let data_count = (header & 0xffff_ffff) as usize;
        for field in 0..pointer_count {
            let at = scan + HEADER_SIZE_IN_WORDS + field;
            let moved = forward(state().next_space[at]);
            state().next_space[at] = moved;
        }
        scan += HEADER_SIZE_IN_WORDS + pointer_count + data_count;
    }
    let current = state();
    current.space.fill(POISON);
    let old = std::mem::replace(&mut current.space, std::mem::take(&mut current.next_space));
    current.old_spaces.push(old);
    current.free_in_words = current.next_free_in_words;
    walk_frames(return_address, stack_pointer, frame_pointer, Visit::Check);
}

extern "C" fn allocate_after_entry(
    pointer_fields: i64,
    data_fields: i64,
    stack_pointer: u64,
    frame_pointer: u64,
    return_address: u64,
) -> *mut u64 {
    collect(return_address, stack_pointer, frame_pointer);
    let state = state();
    let size_in_words = HEADER_SIZE_IN_WORDS + pointer_fields as usize + data_fields as usize;
    assert!(state.free_in_words + size_in_words <= SPACE_SIZE_IN_WORDS, "out of memory");
    // SAFETY: just checked that it fits.
    let object = unsafe { state.space.as_mut_ptr().add(state.free_in_words) };
    state.free_in_words += size_in_words;
    // SAFETY: as above, and a new space starts as all zeros.
    unsafe {
        object.write((pointer_fields as u64) << 32 | data_fields as u64);
        object.add(1).write(0);
    }
    object
}

extern "C" fn collect_after_entry(stack_pointer: u64, frame_pointer: u64, return_address: u64) {
    collect(return_address, stack_pointer, frame_pointer);
}

/// Allocates an object, after collecting. This has to see the registers as they are in its caller.
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn gc_allocate(_pointer_fields: i64, _data_fields: i64) -> *mut u64 {
    core::arch::naked_asm!(
        "mov x2, sp",
        "mov x3, x29",
        "mov x4, x30",
        "stp x29, x30, [sp, #-16]!",
        "mov x29, sp",
        "bl {inner}",
        "ldp x29, x30, [sp], #16",
        "ret",
        inner = sym allocate_after_entry,
    )
}

/// Allocates an object, after collecting. This has to see the registers as they are in its caller.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn gc_allocate(_pointer_fields: i64, _data_fields: i64) -> *mut u64 {
    core::arch::naked_asm!(
        "lea rdx, [rsp + 8]",
        "mov rcx, rbp",
        "mov r8, [rsp]",
        "push rbp",
        "mov rbp, rsp",
        "call {inner}",
        "pop rbp",
        "ret",
        inner = sym allocate_after_entry,
    )
}

/// Collects. This has to see the registers as they are in its caller.
#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn gc_collect() {
    core::arch::naked_asm!(
        "mov x0, sp",
        "mov x1, x29",
        "mov x2, x30",
        "stp x29, x30, [sp, #-16]!",
        "mov x29, sp",
        "bl {inner}",
        "ldp x29, x30, [sp], #16",
        "ret",
        inner = sym collect_after_entry,
    )
}

/// Collects. This has to see the registers as they are in its caller.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn gc_collect() {
    core::arch::naked_asm!(
        "lea rdi, [rsp + 8]",
        "mov rsi, rbp",
        "mov rdx, [rsp]",
        "push rbp",
        "mov rbp, rsp",
        "call {inner}",
        "pop rbp",
        "ret",
        inner = sym collect_after_entry,
    )
}

/// The program says it uses statepoints: a JIT's sections, or what is loaded in this process.
#[unsafe(no_mangle)]
pub extern "C" fn gc_use_stack_maps() {
    let current = state();
    let mut stack_maps = StackMaps::new();
    if current.preset_sections.is_empty() {
        for section in loaded_sections().expect("the stack maps of this program") {
            stack_maps.add_section(section).expect("a good stack map");
        }
    } else {
        for section in std::mem::take(&mut current.preset_sections) {
            // The sections stay for the rest of this process, as the program's code does.
            stack_maps.add_section(Box::leak(section.into_boxed_slice())).expect("a good stack map");
        }
    }
    assert!(stack_maps.call_count() > 0, "this program has no stack map");
    current.finder = RootFinder::StackMaps(stack_maps);
}

/// The program says it uses the shadow stack, whose head this is.
#[unsafe(no_mangle)]
pub extern "C" fn gc_use_shadow_stack(head: *mut *mut StackEntry) {
    state().finder = RootFinder::ShadowStack(head);
}

/// The program says it uses the `ocaml` strategy, whose frame table this is.
///
/// # Safety
/// `table` is a frame table in the format `OcamlGCPrinter.cpp` writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gc_use_frame_table(table: *const u8) {
    let mut descriptors = Vec::new();
    // SAFETY: the caller promises the format: a count, then each descriptor aligned to 8 bytes.
    unsafe {
        let count = table.cast::<u16>().read_unaligned();
        let mut at = table.add(8);
        for _ in 0..count {
            let return_address = at.cast::<u64>().read_unaligned();
            let frame_size_in_bytes = at.add(8).cast::<u16>().read_unaligned();
            let offset_count = at.add(10).cast::<u16>().read_unaligned();
            let offsets = at.add(12).cast::<u16>();
            descriptors.push(FrameDescriptor { return_address, frame_size_in_bytes, offsets, offset_count });
            let size = 12 + usize::from(offset_count) * 2;
            at = at.add((size + 7) & !7);
        }
    }
    // One table per translation unit, and each calls this.
    if let RootFinder::FrameTable(existing) = &mut state().finder {
        descriptors.append(existing);
    }
    descriptors.sort_unstable_by_key(|descriptor| descriptor.return_address);
    state().finder = RootFinder::FrameTable(descriptors);
}

/// The program reports a number.
#[unsafe(no_mangle)]
pub extern "C" fn gc_report(value: i64) {
    let _ = writeln!(state().output, "value {value}");
}

/// The program reports where an object is now.
#[unsafe(no_mangle)]
pub extern "C" fn gc_report_pointer(address: u64) {
    let current = state();
    let in_current_space = inside(&current.space, address);
    let _ = writeln!(current.output, "object at {address:#x} in the current space: {in_current_space}");
}

/// The program finishes, and reports what the collector did.
#[unsafe(no_mangle)]
pub extern "C" fn gc_report_statistics() {
    let current = state();
    let tables = match &current.finder {
        RootFinder::StackMaps(stack_maps) => stack_maps.tables().len(),
        _ => 0,
    };
    let _ = writeln!(
        current.output,
        "collections {}, objects moved {}, roots rewritten {}, derived pointers rewritten {}, frames walked {}, stack map tables {}",
        current.collections,
        current.objects_moved,
        current.roots_rewritten,
        current.derived_pointers_rewritten,
        current.frames_walked,
        tables
    );
}
