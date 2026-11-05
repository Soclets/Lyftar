//! LLVM types, and how Rust says what is known about them (D4, D6, D7, D9).

use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::context::Context;
use crate::raw;
use crate::text::from_owned_message;

/// Makes a lifetime invariant, so that two contexts can never unify (D10).
pub(crate) type Invariant<'context> = PhantomData<fn(&'context ()) -> &'context ()>;

/// What is known about something only when the line runs: checked there, and panicking as in D3 (D4).
#[derive(Clone, Copy, Debug)]
pub struct Dynamic;

/// A width in bits (D6, D7), as in `Integer<Bits<64>>`.
#[derive(Clone, Copy, Debug)]
pub struct Bits<const N: u32>;

/// An integer type: `Integer<Bits<64>>` when the width is known, and `Integer` (of [`Dynamic`] width) when it isn't.
#[derive(Clone, Copy, Debug)]
pub struct Integer<Width = Dynamic>(PhantomData<fn() -> Width>);

/// A floating-point type, which says its format, as in `Float<Ieee754Binary<Bits<64>>>` (D9).
#[derive(Clone, Copy, Debug)]
pub struct Float<Format = Dynamic>(PhantomData<fn() -> Format>);

/// IEEE 754's binary formats (D9).
#[derive(Clone, Copy, Debug)]
pub struct Ieee754Binary<Width>(PhantomData<fn() -> Width>);

/// `bfloat` (D9).
#[derive(Clone, Copy, Debug)]
pub struct BrainFloat;

/// `x86_fp80` (D9).
#[derive(Clone, Copy, Debug)]
pub struct X87Extended;

/// `ppc_fp128` (D9).
#[derive(Clone, Copy, Debug)]
pub struct PowerPcDoubleDouble;

/// A pointer.
#[derive(Clone, Copy, Debug)]
pub struct Pointer;

mod sealed {
    pub trait Sealed {}
}
pub(crate) use sealed::Sealed;

/// What Rust knows about an LLVM type (D4).
pub trait TypeKind: Sealed + 'static {
    /// Whether `raw` is such a type.
    ///
    /// # Safety
    /// `raw` must be a live LLVM type.
    #[doc(hidden)]
    unsafe fn accepts_type(raw: raw::LLVMTypeRef) -> bool;
}

/// A type written fully into the Rust code, so it can be made from the context alone.
pub trait SpecificType: TypeKind {
    #[doc(hidden)]
    fn get_raw(context: raw::LLVMContextRef) -> raw::LLVMTypeRef;
}

/// A type: `Type<'context, Integer<Bits<64>>>` when the Rust code knows it, and `Type<'context>` when it doesn't.
#[repr(transparent)]
pub struct Type<'context, T = Dynamic> {
    pub(crate) raw: raw::LLVMTypeRef,
    marker: PhantomData<(Invariant<'context>, fn() -> T)>,
}

impl<T> Clone for Type<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Type<'_, T> {}
impl<T> PartialEq for Type<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}
impl<T> Eq for Type<'_, T> {}

impl Sealed for Dynamic {}
impl TypeKind for Dynamic {
    unsafe fn accepts_type(_raw: raw::LLVMTypeRef) -> bool {
        true
    }
}

impl Sealed for Integer<Dynamic> {}
impl TypeKind for Integer<Dynamic> {
    unsafe fn accepts_type(raw: raw::LLVMTypeRef) -> bool {
        // SAFETY: the caller promises a live type.
        unsafe { raw::LLVMGetTypeKind(raw) == raw::LLVMTypeKind::LLVMIntegerTypeKind }
    }
}

impl<const N: u32> Sealed for Integer<Bits<N>> {}
impl<const N: u32> TypeKind for Integer<Bits<N>> {
    unsafe fn accepts_type(raw: raw::LLVMTypeRef) -> bool {
        // SAFETY: the caller promises a live type, and the width is only asked of an integer type.
        unsafe {
            raw::LLVMGetTypeKind(raw) == raw::LLVMTypeKind::LLVMIntegerTypeKind && raw::LLVMGetIntTypeWidth(raw) == N
        }
    }
}
impl<const N: u32> SpecificType for Integer<Bits<N>> {
    fn get_raw(context: raw::LLVMContextRef) -> raw::LLVMTypeRef {
        // LLVM's integer widths run from 1 to 2^23 bits, so this fails at `cargo build` when written wrong.
        const { assert!(N >= 1 && N <= (1 << 23), "an LLVM integer is between 1 and 2^23 bits wide") };
        // SAFETY: the context is live, as it came from a `Context`, and N was just checked.
        unsafe { raw::LLVMIntTypeInContext(context, N) }
    }
}

impl Sealed for Pointer {}
impl TypeKind for Pointer {
    unsafe fn accepts_type(raw: raw::LLVMTypeRef) -> bool {
        // SAFETY: the caller promises a live type.
        unsafe { raw::LLVMGetTypeKind(raw) == raw::LLVMTypeKind::LLVMPointerTypeKind }
    }
}

impl<'context, T> Type<'context, T> {
    /// Wraps a raw type.
    ///
    /// # Safety
    /// `raw` must be a live type of this context, of the kind `T` says.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMTypeRef) -> Self {
        Type { raw, marker: PhantomData }
    }

    /// The raw type.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(self) -> raw::LLVMTypeRef {
        self.raw
    }

    /// This type, with nothing known about it in the Rust code.
    #[must_use]
    pub fn to_dynamic(self) -> Type<'context> {
        Type { raw: self.raw, marker: PhantomData }
    }

    /// LLVM's text for this type, such as `i64`.
    #[must_use]
    pub fn print_to_string(self) -> String {
        // SAFETY: the type lives as long as its context.
        from_owned_message(unsafe { raw::LLVMPrintTypeToString(self.raw) })
    }
}

impl<'context> Context<'context> {
    /// The type `T`, when the Rust code spells it out.
    #[must_use]
    pub fn get_type<T: SpecificType>(self) -> Type<'context, T> {
        Type { raw: T::get_raw(self.raw), marker: PhantomData }
    }

    /// A function type.
    ///
    /// # Panics
    /// If the return type or a parameter type can't be used in a function (D3).
    #[must_use]
    #[track_caller]
    pub fn function_type(
        self,
        return_type: Type<'context>,
        parameter_types: &[Type<'context>],
        is_variadic: bool,
    ) -> Type<'context> {
        let Ok(parameter_count) = u32::try_from(parameter_types.len()) else {
            bad_call(format_args!("function_type: {} parameters is too many", parameter_types.len()))
        };
        // SAFETY: LLVM only reads `parameter_count` types; `Type` has the same layout as `LLVMTypeRef`.
        let raw = unsafe {
            raw::LLVMFunctionType(
                return_type.raw,
                parameter_types.as_ptr().cast_mut().cast::<raw::LLVMTypeRef>(),
                parameter_count,
                i32::from(is_variadic),
            )
        };
        Type { raw, marker: PhantomData }
    }
}
