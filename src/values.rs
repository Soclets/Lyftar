//! LLVM values (D4, D11).

use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::block::Block;
use crate::raw;
use crate::text::{c_string, from_owned_message};
use crate::types::{Dynamic, Invariant, Pointer, Sealed, Type, TypeKind};

/// A value: as specific as the code knows (D4), belonging to its context (invariant, D10) and borrowing its module
/// (covariant, D11), so that holding one across a delete or a pass run doesn't compile and constants, which belong to
/// the context, can be used with any module.
#[repr(transparent)]
pub struct Value<'context, 'module, T = Dynamic> {
    pub(crate) raw: raw::LLVMValueRef,
    marker: PhantomData<(Invariant<'context>, &'module (), fn() -> T)>,
}

impl<T> Clone for Value<'_, '_, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Value<'_, '_, T> {}

/// A function. It is a value (D4), and its LLVM type is a pointer.
#[derive(Clone, Copy, Debug)]
pub struct Function;

/// What Rust knows about a value (D4).
pub trait ValueKind: Sealed + 'static {
    /// What is known about this value's LLVM type.
    type LlvmType: TypeKind;

    /// Whether `raw` is such a value.
    ///
    /// # Safety
    /// `raw` must be a live LLVM value.
    #[doc(hidden)]
    unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool;
}

impl<T: TypeKind> ValueKind for T {
    type LlvmType = T;

    unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool {
        // SAFETY: the caller promises a live value, and a live value has a live type.
        unsafe { T::accepts_type(raw::LLVMTypeOf(raw)) }
    }
}

impl<'context, 'module, T> Value<'context, 'module, T> {
    /// Wraps a raw value.
    ///
    /// # Safety
    /// `raw` must be a live value of this context, of the kind `T` says, and stay live for `'module`.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMValueRef) -> Self {
        Value { raw, marker: PhantomData }
    }

    /// The raw value.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(self) -> raw::LLVMValueRef {
        self.raw
    }

    /// This value, with nothing known about it in the Rust code.
    #[must_use]
    pub fn to_dynamic(self) -> Value<'context, 'module> {
        Value { raw: self.raw, marker: PhantomData }
    }

    /// LLVM's text for this value.
    #[must_use]
    pub fn print_to_string(self) -> String {
        // SAFETY: the value is live for `'module`.
        from_owned_message(unsafe { raw::LLVMPrintValueToString(self.raw) })
    }
}

impl<'context, 'module, T: ValueKind> Value<'context, 'module, T> {
    /// This value's type.
    #[must_use]
    pub fn get_type(self) -> Type<'context, T::LlvmType> {
        // SAFETY: the value is live, and `T` says what its type is.
        unsafe { Type::from_raw(raw::LLVMTypeOf(self.raw)) }
    }

    /// This value as a `U`, if it is one (D4).
    #[must_use]
    pub fn as_kind<U: ValueKind>(self) -> Option<Value<'context, 'module, U>> {
        // SAFETY: the value is live for `'module`.
        unsafe { U::accepts_value(self.raw) }.then_some(Value { raw: self.raw, marker: PhantomData })
    }

    // trace: D4
    /// This value as a `U`, checked here.
    ///
    /// # Panics
    /// If it isn't one (D3).
    #[must_use]
    #[track_caller]
    pub fn checked_cast<U: ValueKind>(self) -> Value<'context, 'module, U> {
        match self.as_kind::<U>() {
            Some(value) => value,
            None => bad_call(format_args!(
                "checked_cast: {} is not a {}",
                self.print_to_string(),
                std::any::type_name::<U>()
            )),
        }
    }
}

impl Sealed for Function {}
impl ValueKind for Function {
    type LlvmType = Pointer;

    unsafe fn accepts_value(raw: raw::LLVMValueRef) -> bool {
        // SAFETY: the caller promises a live value.
        unsafe { !raw::LLVMIsAFunction(raw).is_null() }
    }
}

impl<'context, 'module> Value<'context, 'module, Function> {
    /// Adds a new block at the end of this function.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn append_block(self, name: &str) -> Block<'context, 'module> {
        let name = c_string(name, "append_block");
        // SAFETY: the function is live for `'module`, and `name` is a valid C string for the call.
        unsafe {
            let context = raw::LLVMGetValueContext(self.raw);
            Block::from_raw(raw::LLVMAppendBasicBlockInContext(context, self.raw, name.as_ptr()))
        }
    }

    /// The parameter at `index`.
    ///
    /// # Panics
    /// If the function has no such parameter (D3).
    #[must_use]
    #[track_caller]
    pub fn parameter(self, index: u32) -> Value<'context, 'module> {
        // SAFETY: the function is live for `'module`.
        let count = unsafe { raw::LLVMCountParams(self.raw) };
        if !crate::checks::index_in_bounds(index, count) {
            bad_call(format_args!("parameter: index {index} is out of bounds for a function of {count} parameters"));
        }
        // SAFETY: the index was just checked.
        unsafe { Value::from_raw(raw::LLVMGetParam(self.raw, index)) }
    }
}
