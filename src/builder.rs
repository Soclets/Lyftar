//! The instruction builder.

use std::ffi::CStr;
use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::block::Block;
use crate::checks;
use crate::raw;
use crate::shim;
use crate::types::{Bits, Dynamic, Integer, Invariant};
use crate::values::{Value, ValueKind};

/// Builds instructions. It only exists placed in a block, so an unset position can't happen (D3).
pub struct Builder<'context, 'module> {
    raw: raw::LLVMBuilderRef,
    module: raw::LLVMModuleRef,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl Drop for Builder<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the builder.
        unsafe { raw::LLVMDisposeBuilder(self.raw) };
    }
}

/// An integer operand, and what a build call has to check about it when the line runs.
pub trait IntegerOperand: ValueKind {
    #[doc(hidden)]
    fn check_operands(operation: &'static str, left: raw::LLVMValueRef, right: raw::LLVMValueRef);
}

impl<const N: u32> IntegerOperand for Integer<Bits<N>> {
    // trace: D24
    // Both types are spelled out and the same, so `cargo build` already checked them.
    #[inline(always)]
    fn check_operands(_operation: &'static str, _left: raw::LLVMValueRef, _right: raw::LLVMValueRef) {}
}

impl IntegerOperand for Dynamic {
    #[inline]
    #[track_caller]
    fn check_operands(operation: &'static str, left: raw::LLVMValueRef, right: raw::LLVMValueRef) {
        // SAFETY: both values are live, because each came from a `Value` that borrows its module.
        let (left_type, right_type) = unsafe { (raw::LLVMTypeOf(left), raw::LLVMTypeOf(right)) };
        if !checks::same_identity(left_type as usize, right_type as usize) {
            // SAFETY: the same two live values.
            let (left_value, right_value) =
                unsafe { (crate::Value::<Dynamic>::from_raw(left), crate::Value::<Dynamic>::from_raw(right)) };
            bad_call(format_args!(
                "{operation}: the operands have different types, {} and {}",
                left_value.get_type().print_to_string(),
                right_value.get_type().print_to_string()
            ));
        }
    }
}

impl<'context, 'module> Builder<'context, 'module> {
    /// A builder placed at the end of `block`.
    #[must_use]
    pub fn at_end_of(block: Block<'context, 'module>) -> Self {
        // SAFETY: the block is live for `'module`, is in a function, and that function is in a module.
        unsafe {
            let function = raw::LLVMGetBasicBlockParent(block.raw);
            let module = raw::LLVMGetGlobalParent(function);
            let builder = raw::LLVMCreateBuilderInContext(raw::LLVMGetModuleContext(module));
            raw::LLVMPositionBuilderAtEnd(builder, block.raw);
            Builder { raw: builder, module, marker: PhantomData }
        }
    }

    #[inline]
    #[track_caller]
    fn check_module<T>(&self, operation: &'static str, value: Value<'context, 'module, T>) {
        // SAFETY: both are live.
        if !unsafe { shim::lyftar_value_belongs_to_module(value.raw, self.module) } {
            bad_call(format_args!(
                "{operation}: {} belongs to another module than this builder's",
                value.print_to_string()
            ));
        }
    }

    #[inline]
    #[track_caller]
    fn check_open(&self, operation: &'static str) {
        // SAFETY: the builder is live and placed.
        if !unsafe { shim::lyftar_builder_is_open(self.raw) } {
            bad_call(format_args!("{operation}: the block already ends in a terminator"));
        }
    }

    /// Adds two integers, wrapping on overflow.
    ///
    /// # Panics
    /// If the operands belong to another module, or their types differ, or the block is already terminated (D3).
    #[track_caller]
    pub fn integer_add<T: IntegerOperand>(
        &self,
        left: Value<'context, 'module, T>,
        right: Value<'context, 'module, T>,
        name: &str,
    ) -> Value<'context, 'module, T> {
        self.check_open("integer_add");
        self.check_module("integer_add", left);
        self.check_module("integer_add", right);
        T::check_operands("integer_add", left.raw, right.raw);
        // SAFETY: the checks above are what LLVM requires of an add.
        let result = unsafe { raw::LLVMBuildAdd(self.raw, left.raw, right.raw, EMPTY.as_ptr()) };
        set_name(result, name);
        // SAFETY: an add of two `T`s is a `T`, live for `'module`.
        unsafe { Value::from_raw(result) }
    }

    /// Returns `value` from the function.
    ///
    /// # Panics
    /// If the value belongs to another module, or the block is already terminated (D3).
    #[track_caller]
    pub fn return_value<T>(&self, value: Value<'context, 'module, T>) -> Value<'context, 'module> {
        self.check_open("return_value");
        self.check_module("return_value", value);
        // SAFETY: the checks above are what LLVM requires.
        unsafe { Value::from_raw(raw::LLVMBuildRet(self.raw, value.raw)) }
    }
}

const EMPTY: &CStr = c"";

// trace: D24
/// Names a built value without allocating: `LLVMSetValueName2` takes a length, where the build calls want a C string.
/// A build call can give back a constant or its operand, which must not be renamed.
#[inline]
fn set_name(value: raw::LLVMValueRef, name: &str) {
    if !name.is_empty() {
        // SAFETY: the value is live, and LLVM reads `name.len()` bytes of `name`.
        unsafe {
            if !raw::LLVMIsAInstruction(value).is_null() {
                raw::LLVMSetValueName2(value, name.as_ptr().cast(), name.len());
            }
        }
    }
}
