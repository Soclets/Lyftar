//! Lyftar's own checks that stand between safe Rust and an `unsafe` LLVM call, proven with Verus (D21, D22).
//!
//! To rustc this file is plain Rust with no dependency; the proofs only exist when Verus compiles it, since only Verus
//! sets `verus_keep_ghost`. `tests/decision_22_proofs.rs` runs them. What LLVM itself does is assumed, and those assumptions
//! are written down in `ASSUMPTIONS.md`.

// Verus reads this file, and knows plain arithmetic rather than these library calls.
#![allow(
    clippy::manual_range_contains,
    clippy::manual_is_power_of_two,
    clippy::bool_to_int_with_if,
    clippy::cast_lossless
)]

// trace: D21
#[cfg(verus_keep_ghost)]
use vstd::prelude::*;

#[cfg(verus_keep_ghost)]
verus! {

/// An index into something of `count` elements is in bounds.
pub open spec fn in_bounds(index: u32, count: u32) -> bool {
    index < count
}

}

// trace: D22
/// Whether `index` is a valid index into something of `count` elements.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result == in_bounds(index, count)))]
pub(crate) fn index_in_bounds(index: u32, count: u32) -> bool {
    index < count
}

// trace: D22
/// Whether two LLVM objects are the same one. LLVM uniques types in a context, so two values have the same type
/// exactly when their type pointers are equal (assumption in `ASSUMPTIONS.md`).
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result == (left == right)))]
pub(crate) fn same_identity(left: usize, right: usize) -> bool {
    left == right
}

#[cfg(verus_keep_ghost)]
verus! {

/// LLVM's integer widths run from 1 to 2^23 bits.
pub open spec fn valid_integer_width(width_in_bits: u32) -> bool {
    1 <= width_in_bits <= 8388608
}

}

// trace: D22
/// Whether LLVM has an integer of this width.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result == valid_integer_width(width_in_bits)))]
pub(crate) fn integer_width_is_valid(width_in_bits: u32) -> bool {
    1 <= width_in_bits && width_in_bits <= 8_388_608
}

#[cfg(verus_keep_ghost)]
verus! {

/// A power of two has exactly one bit set: the only form LLVM's `Align` can hold.
pub open spec fn power_of_two(value: u32) -> bool {
    value != 0 && (value & sub(value, 1)) == 0
}

/// A digit's value in a radix up to 36.
pub open spec fn digit_value(digit: u8) -> int {
    if 48 <= digit <= 57 { digit - 48 }
    else if 97 <= digit <= 122 { digit - 97 + 10 }
    else if 65 <= digit <= 90 { digit - 65 + 10 }
    else { 36 }
}

/// LLVM's `StringRef::getAsInteger` radixes for `ConstantInt`: 2, 8, 10, 16 and 36.
pub open spec fn valid_radix(radix: u8) -> bool {
    radix == 2 || radix == 8 || radix == 10 || radix == 16 || radix == 36
}

/// All the digits at or after `start` are digits of the radix.
pub open spec fn digits_valid_from(text: Seq<u8>, start: int, radix: u8) -> bool {
    forall|index: int| start <= index < text.len() ==> digit_value(#[trigger] text[index]) < radix as int
}

/// Where the digits start, after an optional sign.
pub open spec fn digits_start(text: Seq<u8>) -> int {
    if text.len() > 0 && (text[0] == 45 || text[0] == 43) { 1 } else { 0 }
}

/// An optional sign, then at least one digit, all of them digits of the radix.
pub open spec fn valid_integer_text(text: Seq<u8>, radix: u8) -> bool {
    text.len() > 0 && digits_start(text) < text.len() && digits_valid_from(text, digits_start(text), radix)
}

}

// trace: D22
/// Whether `value` is a power of two, the only alignment LLVM can hold.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result == power_of_two(value)))]
pub(crate) fn is_power_of_two(value: u32) -> bool {
    value != 0 && (value & (value - 1)) == 0
}

// trace: D22
/// Whether a target width is strictly narrower than a source width.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result == (target_in_bits < source_in_bits)))]
pub(crate) fn is_narrowing(source_in_bits: u32, target_in_bits: u32) -> bool {
    target_in_bits < source_in_bits
}

// trace: D22
/// Whether there are enough words for what's needed.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result == (needed > 0 && given >= needed)))]
pub(crate) fn enough_words(given: usize, needed: usize) -> bool {
    needed > 0 && given >= needed
}

// trace: D22
/// Whether `size_in_bytes` bytes are a whole number of elements of `element_size_in_bits`, which is zero for something
/// that can't be an element.
#[cfg_attr(verus_keep_ghost, verus_spec(result =>
    ensures result == (element_size_in_bits > 0 && (size_in_bytes as int * 8) % element_size_in_bits as int == 0)
))]
pub(crate) fn data_fits_elements(size_in_bytes: u64, element_size_in_bits: u64) -> bool {
    if element_size_in_bits == 0 {
        return false;
    }
    (u128::from(size_in_bytes) * 8) % u128::from(element_size_in_bits) == 0
}

// trace: D22
/// Whether `text` is a number in `radix`: a radix LLVM takes, an optional sign, and at least one digit, all of them
/// digits of the radix. `LLVMConstIntOfStringAndSize` aborts on anything else.
#[cfg_attr(verus_keep_ghost, verus_spec(result =>
    ensures result == (valid_radix(radix) && valid_integer_text(text@, radix)),
))]
pub(crate) fn integer_text_is_valid(text: &[u8], radix: u8) -> bool {
    if !(radix == 2 || radix == 8 || radix == 10 || radix == 16 || radix == 36) {
        return false;
    }
    if text.is_empty() {
        return false;
    }
    let mut index: usize = if text[0] == 45 || text[0] == 43 { 1 } else { 0 };
    if index == text.len() {
        return false;
    }
    #[cfg_attr(verus_keep_ghost, verus_spec(
        invariant
            valid_radix(radix),
            text@.len() > 0,
            digits_start(text@) <= index <= text@.len(),
            digits_start(text@) < text@.len(),
            forall|earlier: int| digits_start(text@) <= earlier < index ==>
                digit_value(#[trigger] text@[earlier]) < radix as int,
        decreases text@.len() - index,
    ))]
    while index < text.len() {
        let digit = text[index];
        let value: u32 = if 48 <= digit && digit <= 57 {
            (digit - 48) as u32
        } else if 97 <= digit && digit <= 122 {
            (digit - 97) as u32 + 10
        } else if 65 <= digit && digit <= 90 {
            (digit - 65) as u32 + 10
        } else {
            36
        };
        if value >= radix as u32 {
            return false;
        }
        index += 1;
    }
    true
}

// trace: D22, D11
/// Whether a list entry can still be trusted: the module's generation changes on every exclusive operation and is
/// unique across all modules, so if it still matches, the entry is from this module and nothing has deleted its value.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result == (entry_generation == module_generation)))]
pub(crate) fn entry_is_current(entry_generation: u64, module_generation: u64) -> bool {
    entry_generation == module_generation
}

#[cfg(verus_keep_ghost)]
verus! {

/// No two neighbours of this list are the same.
pub open spec fn neighbours_differ(items: Seq<usize>) -> bool {
    forall|index: int| 0 <= index && index + 1 < items.len() ==> #[trigger] items[index] != items[index + 1]
}

/// The list is sorted.
pub open spec fn is_sorted(items: Seq<usize>) -> bool {
    forall|left: int, right: int| 0 <= left <= right < items.len() ==> #[trigger] items[left] <= #[trigger] items[right]
}

/// No two places in the list hold the same item.
pub open spec fn all_distinct(items: Seq<usize>) -> bool {
    forall|left: int, right: int| 0 <= left < right < items.len() ==> #[trigger] items[left] != #[trigger] items[right]
}

proof fn sorted_neighbours_differ_means_distinct(items: Seq<usize>)
    requires is_sorted(items), neighbours_differ(items)
    ensures all_distinct(items)
{
    assert forall|left: int, right: int| 0 <= left < right < items.len() implies #[trigger] items[left] != #[trigger] items[right] by {
        assert(items[left] != items[left + 1]);
        assert(items[left] <= items[left + 1]);
        assert(items[left + 1] <= items[right]);
    }
}

}

// trace: D22, D11
/// Whether a sorted list has no duplicates. The sort is `slice::sort_unstable`, which is assumed to sort
/// (`ASSUMPTIONS.md`).
#[cfg_attr(verus_keep_ghost, verus_spec(result =>
    ensures result == neighbours_differ(sorted@),
))]
pub(crate) fn sorted_has_no_duplicates(sorted: &[usize]) -> bool {
    let mut index = 1;
    #[cfg_attr(verus_keep_ghost, verus_spec(
        invariant
            1 <= index,
            forall|earlier: int| 0 <= earlier && earlier + 1 < index && earlier + 1 < sorted@.len() ==>
                #[trigger] sorted@[earlier] != sorted@[earlier + 1],
        decreases sorted@.len() - index,
    ))]
    while index < sorted.len() {
        if sorted[index - 1] == sorted[index] {
            return false;
        }
        index += 1;
    }
    true
}

// trace: D22
/// Whether an index is in bounds of an aggregate, whose element count LLVM holds as 64 bits and which is zero for a type
/// that isn't an aggregate.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result == (index < count)))]
pub(crate) fn aggregate_index_in_bounds(index: u64, count: u64) -> bool {
    index < count
}

// trace: D22
/// Whether LLVM can hold this calling convention: it keeps 10 bits for one.
#[cfg_attr(verus_keep_ghost, verus_spec(result => ensures result == (number <= 1023)))]
pub(crate) fn calling_convention_is_valid(number: u32) -> bool {
    number <= 1023
}
