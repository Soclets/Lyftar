//! Lyftar's own checks that stand between safe Rust and an `unsafe` LLVM call, proven with Verus (D21, D22).
//!
//! To rustc this file is plain Rust with no dependency; the proofs only exist when Verus compiles it, since only Verus
//! sets `verus_keep_ghost`. Run them with `tools/verus.sh`. What LLVM itself does is assumed, and those assumptions are
//! written down in `ASSUMPTIONS.md`.

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
