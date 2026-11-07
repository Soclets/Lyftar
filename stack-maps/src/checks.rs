//! The bounds checks of the stack map reader, proven with Verus (D22, D45).
//!
//! The reader runs inside a language's finished program, so it gets its own small file with no dependency, as `checks.rs` is for
//! Lyftar. To rustc this is plain Rust; the proofs only exist when Verus compiles it. `tests/decision_22_proofs.rs` runs them.

// Verus reads this file, and knows plain arithmetic rather than these library calls.
#![allow(clippy::manual_range_contains, clippy::manual_is_multiple_of, clippy::cast_lossless)]

// trace: D21, D45
#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn range_is_inside(offset: usize, size: usize, total: usize) -> bool {
    offset as int + size as int <= total as int
}

pub open spec fn is_aligned_to_8(value: int) -> bool {
    value % 8 == 0
}

}

// trace: D22, D45
/// Whether `size` bytes starting at `offset` are inside something of `total` bytes, in arithmetic that can't overflow.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result == range_is_inside(offset, size, total)))]
pub(crate) fn range_is_inside_total(offset: usize, size: usize, total: usize) -> bool {
    size <= total && offset <= total - size
}

#[cfg(verus_keep_ghost)]
verus! {

pub open spec fn aligned_to_8(offset: usize, result: Option<usize>) -> bool {
    match result {
        Some(aligned) => aligned as int >= offset as int
            && (aligned as int) < offset as int + 8
            && is_aligned_to_8(aligned as int),
        None => offset as int + 8 > usize::MAX as int,
    }
}

}

// trace: D22, D45
/// The next 8-byte boundary from `offset`, unless it is so near the end of memory that it might overflow.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures aligned_to_8(offset, result)))]
pub(crate) fn align_to_8(offset: usize) -> Option<usize> {
    if offset > usize::MAX - 8 {
        return None;
    }
    Some(offset + (8 - offset % 8) % 8)
}

// trace: D22, D45
/// How many bytes the header, function list and constant list of a stack map take. Counts of 32 bits can't overflow this.
#[cfg_attr(verus_keep_ghost, verus_spec(result =>
    ensures result as int == 16 + 24 * function_count as int + 8 * constant_count as int
))]
pub(crate) fn lists_size_in_bytes(function_count: u32, constant_count: u32) -> u64 {
    16 + 24 * function_count as u64 + 8 * constant_count as u64
}

// trace: D22, D45
/// How many bytes a record's fixed fields and locations take. A count of 16 bits can't overflow this.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result as int == 16 + 12 * location_count as int))]
pub(crate) fn locations_end_in_record(location_count: u16) -> usize {
    16 + 12 * location_count as usize
}

// trace: D22, D45
/// How many bytes live-outs take, with the padding and count before them. A count of 16 bits can't overflow this.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result as int == 4 + 4 * live_out_count as int))]
pub(crate) fn live_outs_size_in_bytes(live_out_count: u16) -> usize {
    4 + 4 * live_out_count as usize
}

// trace: D22, D45
/// Whether a count of records said so far, and one more function's, add up without going past the header's count.
#[cfg_attr(verus_keep_ghost, verus_spec(result =>
    ensures result == (records_so_far as int + more as int <= total as int)
))]
pub(crate) fn record_count_fits(records_so_far: u64, more: u64, total: u64) -> bool {
    more <= total && records_so_far <= total - more
}

// trace: D22, D45
/// Whether an index is inside a list of `count` things.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result == (index < count)))]
pub(crate) fn index_is_inside(index: u32, count: u32) -> bool {
    index < count
}
