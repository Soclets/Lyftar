//! LLVM types, and how Rust says what is known about them (D4, D6, D7, D9, D30).

use std::ffi::c_uint;
use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::checks;
use crate::context::Context;
use crate::enums::TypeKind;
use crate::raw;
use crate::text::{c_string, from_borrowed, from_owned_message};
use crate::values::Value;

/// Makes a lifetime invariant, so that two contexts can never unify (D10).
pub(crate) type Invariant<'context> = PhantomData<fn(&'context ()) -> &'context ()>;

/// What is known about something only when the line runs: checked there, and panicking as in D3 (D4).
#[derive(Clone, Copy, Debug)]
pub struct Dynamic;

/// A width in bits (D6, D7), as in `Integer<Bits<64>>`.
#[derive(Clone, Copy, Debug)]
// trace: D6
pub struct Bits<const N: u32>;

/// An integer type: `Integer<Bits<64>>` when the width is known, and `Integer` (of [`Dynamic`] width) when it isn't.
#[derive(Clone, Copy, Debug)]
// trace: D7
pub struct Integer<Width = Dynamic>(PhantomData<fn() -> Width>);

/// LLVM 23's byte type, which holds raw memory in a value: `Byte<Bits<8>>` when the width is known, and `Byte` (of
/// [`Dynamic`] width) when it isn't.
#[derive(Clone, Copy, Debug)]
// trace: D30
pub struct Byte<Width = Dynamic>(PhantomData<fn() -> Width>);

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
pub trait TypeMarker: Sealed + 'static {
    /// Whether two values of this marker always have the same LLVM type, so `cargo build` has already checked it (D24).
    #[doc(hidden)]
    const EXACT: bool;

    /// Whether `raw` is such a type.
    ///
    /// # Safety
    /// `raw` must be a live LLVM type.
    #[doc(hidden)]
    unsafe fn accepts_type(raw: raw::LLVMTypeRef) -> bool;
}

/// A type written fully into the Rust code, so it can be made from the context alone.
pub trait SpecificType: TypeMarker {
    #[doc(hidden)]
    fn get_raw(context: Context<'_>) -> raw::LLVMTypeRef;
}

/// A floating-point format that the Rust code spells out, and so how many bits a value of it takes.
pub trait FloatFormat: 'static {
    /// The format's size in bits, which orders the casts between formats.
    const SIZE_IN_BITS: u32;
    #[doc(hidden)]
    const RAW_KIND: raw::LLVMTypeKind;
    #[doc(hidden)]
    fn get_raw(context: Context<'_>) -> raw::LLVMTypeRef;
}

/// Declares a floating-point format.
macro_rules! float_format {
    ($format:ty, $size:literal, $kind:ident, $get:ident) => {
        impl FloatFormat for $format {
            const SIZE_IN_BITS: u32 = $size;
            const RAW_KIND: raw::LLVMTypeKind = raw::LLVMTypeKind::$kind;
            fn get_raw(context: Context<'_>) -> raw::LLVMTypeRef {
                // SAFETY: the context is live, as it is a `Context`.
                unsafe { raw::$get(context.raw) }
            }
        }
    };
}
// trace: D9
float_format!(Ieee754Binary<Bits<16>>, 16, LLVMHalfTypeKind, LLVMHalfTypeInContext);
float_format!(Ieee754Binary<Bits<32>>, 32, LLVMFloatTypeKind, LLVMFloatTypeInContext);
float_format!(Ieee754Binary<Bits<64>>, 64, LLVMDoubleTypeKind, LLVMDoubleTypeInContext);
float_format!(Ieee754Binary<Bits<128>>, 128, LLVMFP128TypeKind, LLVMFP128TypeInContext);
float_format!(BrainFloat, 16, LLVMBFloatTypeKind, LLVMBFloatTypeInContext);
float_format!(X87Extended, 80, LLVMX86_FP80TypeKind, LLVMX86FP80TypeInContext);
float_format!(PowerPcDoubleDouble, 128, LLVMPPC_FP128TypeKind, LLVMPPCFP128TypeInContext);

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
impl<T, U> PartialEq<Type<'_, U>> for Type<'_, T> {
    fn eq(&self, other: &Type<'_, U>) -> bool {
        self.raw == other.raw
    }
}
impl<T> Eq for Type<'_, T> {}
impl<T> std::fmt::Debug for Type<'_, T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.print_to_string())
    }
}

impl Sealed for Dynamic {}
impl TypeMarker for Dynamic {
    const EXACT: bool = false;
    unsafe fn accepts_type(_raw: raw::LLVMTypeRef) -> bool {
        true
    }
}

impl Sealed for Integer<Dynamic> {}
impl TypeMarker for Integer<Dynamic> {
    const EXACT: bool = false;
    unsafe fn accepts_type(raw: raw::LLVMTypeRef) -> bool {
        // SAFETY: the caller promises a live type.
        unsafe { raw::LLVMGetTypeKind(raw) == raw::LLVMTypeKind::LLVMIntegerTypeKind }
    }
}

impl<const N: u32> Sealed for Integer<Bits<N>> {}
impl<const N: u32> TypeMarker for Integer<Bits<N>> {
    const EXACT: bool = true;
    unsafe fn accepts_type(raw: raw::LLVMTypeRef) -> bool {
        // SAFETY: the caller promises a live type, and the width is only asked of an integer type.
        unsafe {
            raw::LLVMGetTypeKind(raw) == raw::LLVMTypeKind::LLVMIntegerTypeKind && raw::LLVMGetIntTypeWidth(raw) == N
        }
    }
}
impl<const N: u32> SpecificType for Integer<Bits<N>> {
    fn get_raw(context: Context<'_>) -> raw::LLVMTypeRef {
        // LLVM's integer widths run from 1 to 2^23 bits, so this fails at `cargo build` when written wrong.
        const { assert!(N >= 1 && N <= (1 << 23), "an LLVM integer is between 1 and 2^23 bits wide") };
        // SAFETY: the context is live, as it is a `Context`, and N was just checked.
        unsafe { raw::LLVMIntTypeInContext(context.raw, N) }
    }
}

impl Sealed for Byte<Dynamic> {}
impl TypeMarker for Byte<Dynamic> {
    const EXACT: bool = false;
    unsafe fn accepts_type(raw: raw::LLVMTypeRef) -> bool {
        // SAFETY: the caller promises a live type.
        unsafe { raw::LLVMGetTypeKind(raw) == raw::LLVMTypeKind::LLVMByteTypeKind }
    }
}

impl<const N: u32> Sealed for Byte<Bits<N>> {}
impl<const N: u32> TypeMarker for Byte<Bits<N>> {
    const EXACT: bool = true;
    unsafe fn accepts_type(raw: raw::LLVMTypeRef) -> bool {
        // SAFETY: the caller promises a live type, and the width is only asked of a byte type.
        unsafe {
            raw::LLVMGetTypeKind(raw) == raw::LLVMTypeKind::LLVMByteTypeKind && raw::LLVMGetByteTypeWidth(raw) == N
        }
    }
}
// trace: D30
impl<const N: u32> SpecificType for Byte<Bits<N>> {
    fn get_raw(context: Context<'_>) -> raw::LLVMTypeRef {
        // LLVM's byte widths run from 1 to 2^23 bits, so this fails at `cargo build` when written wrong.
        const { assert!(N >= 1 && N <= (1 << 23), "an LLVM byte is between 1 and 2^23 bits wide") };
        // SAFETY: the context is live, as it is a `Context`, and N was just checked.
        unsafe { raw::LLVMByteTypeInContext(context.raw, N) }
    }
}

impl Sealed for Float<Dynamic> {}
impl TypeMarker for Float<Dynamic> {
    const EXACT: bool = false;
    unsafe fn accepts_type(raw: raw::LLVMTypeRef) -> bool {
        // SAFETY: the caller promises a live type.
        is_float_kind(unsafe { raw::LLVMGetTypeKind(raw) })
    }
}

impl<F: FloatFormat> Sealed for Float<F> {}
impl<F: FloatFormat> TypeMarker for Float<F> {
    const EXACT: bool = true;
    unsafe fn accepts_type(raw: raw::LLVMTypeRef) -> bool {
        // SAFETY: the caller promises a live type.
        unsafe { raw::LLVMGetTypeKind(raw) == F::RAW_KIND }
    }
}
impl<F: FloatFormat> SpecificType for Float<F> {
    fn get_raw(context: Context<'_>) -> raw::LLVMTypeRef {
        F::get_raw(context)
    }
}

pub(crate) fn is_float_kind(kind: raw::LLVMTypeKind) -> bool {
    use raw::LLVMTypeKind as K;
    matches!(
        kind,
        K::LLVMHalfTypeKind
            | K::LLVMBFloatTypeKind
            | K::LLVMFloatTypeKind
            | K::LLVMDoubleTypeKind
            | K::LLVMX86_FP80TypeKind
            | K::LLVMFP128TypeKind
            | K::LLVMPPC_FP128TypeKind
    )
}

impl Sealed for Pointer {}
impl TypeMarker for Pointer {
    const EXACT: bool = false;
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

    /// This type as a `U`, if it is one (D4).
    #[must_use]
    pub fn as_kind<U: TypeMarker>(self) -> Option<Type<'context, U>> {
        // SAFETY: the type lives as long as its context.
        unsafe { U::accepts_type(self.raw) }.then_some(Type { raw: self.raw, marker: PhantomData })
    }

    // trace: D4
    /// This type as a `U`, checked here.
    ///
    /// # Panics
    /// If it isn't one (D3).
    #[must_use]
    #[track_caller]
    pub fn checked_cast<U: TypeMarker>(self) -> Type<'context, U> {
        match self.as_kind::<U>() {
            Some(cast) => cast,
            None => bad_call(format_args!("checked_cast: {} is not a {}", self.describe(), std::any::type_name::<U>())),
        }
    }

    /// What a panic calls this type.
    #[must_use]
    pub(crate) fn describe(self) -> String {
        self.print_to_string()
    }

    /// LLVM's text for this type, such as `i64`.
    #[must_use]
    pub fn print_to_string(self) -> String {
        // SAFETY: the type lives as long as its context.
        from_owned_message(unsafe { raw::LLVMPrintTypeToString(self.raw) })
    }

    /// What kind of type this is.
    #[must_use]
    pub fn kind(self) -> TypeKind {
        // SAFETY: the type lives as long as its context.
        TypeKind::from_raw(unsafe { raw::LLVMGetTypeKind(self.raw) })
    }

    /// Whether this type has a size.
    #[must_use]
    pub fn is_sized(self) -> bool {
        // SAFETY: the type lives as long as its context.
        unsafe { raw::LLVMTypeIsSized(self.raw) != 0 }
    }

    /// The context this type belongs to.
    #[must_use]
    pub fn context(self) -> Context<'context> {
        // SAFETY: the type's context is live for `'context`.
        unsafe { Context::from_raw(raw::LLVMGetTypeContext(self.raw)) }
    }

    pub(crate) fn raw_kind(self) -> raw::LLVMTypeKind {
        // SAFETY: the type lives as long as its context.
        unsafe { raw::LLVMGetTypeKind(self.raw) }
    }

    #[track_caller]
    fn require_kind(self, wanted: raw::LLVMTypeKind, operation: &str, what: &str) {
        if self.raw_kind() != wanted {
            bad_call(format_args!("{operation}: {} is not {what}", self.describe()));
        }
    }

    /// A pointer-sized integer constant holding this type's size in bytes, as a constant expression.
    ///
    /// # Panics
    /// If the type has no size (D3).
    #[must_use]
    #[track_caller]
    pub fn size_of(self) -> Value<'context, 'static> {
        if !self.is_sized() {
            bad_call(format_args!("size_of: {} has no size", self.describe()));
        }
        // SAFETY: the type was just checked to be sized, and the result belongs to the context.
        unsafe { Value::from_raw(raw::LLVMSizeOf(self.raw)) }
    }

    /// A pointer-sized integer constant holding this type's alignment in bytes, as a constant expression.
    ///
    /// # Panics
    /// If the type has no size, so no alignment (D3).
    #[must_use]
    #[track_caller]
    pub fn alignment_of(self) -> Value<'context, 'static> {
        if !self.is_sized() {
            bad_call(format_args!("alignment_of: {} has no size", self.describe()));
        }
        // SAFETY: the type was just checked to be sized, and the result belongs to the context.
        unsafe { Value::from_raw(raw::LLVMAlignOf(self.raw)) }
    }

    /// The types this type directly contains: a function's return and parameter types, an aggregate's elements.
    #[must_use]
    pub fn contained_types(self) -> Vec<Type<'context>> {
        // SAFETY: the type is live, and the buffer has room for all of them.
        unsafe {
            let count = raw::LLVMGetNumContainedTypes(self.raw);
            let mut types = vec![std::ptr::null_mut(); count as usize];
            raw::LLVMGetSubtypes(self.raw, types.as_mut_ptr());
            types.into_iter().map(|contained| Type::from_raw(contained)).collect()
        }
    }

    // ---- Functions ----

    /// Whether this function type is variadic.
    ///
    /// # Panics
    /// If this isn't a function type (D3).
    #[must_use]
    #[track_caller]
    pub fn is_variadic(self) -> bool {
        self.require_kind(raw::LLVMTypeKind::LLVMFunctionTypeKind, "is_variadic", "a function type");
        // SAFETY: just checked to be a function type.
        unsafe { raw::LLVMIsFunctionVarArg(self.raw) != 0 }
    }

    /// This function type's return type.
    ///
    /// # Panics
    /// If this isn't a function type (D3).
    #[must_use]
    #[track_caller]
    pub fn return_type(self) -> Type<'context> {
        self.require_kind(raw::LLVMTypeKind::LLVMFunctionTypeKind, "return_type", "a function type");
        // SAFETY: just checked to be a function type.
        unsafe { Type::from_raw(raw::LLVMGetReturnType(self.raw)) }
    }

    /// This function type's parameter types.
    ///
    /// # Panics
    /// If this isn't a function type (D3).
    #[must_use]
    #[track_caller]
    pub fn parameter_types(self) -> Vec<Type<'context>> {
        self.require_kind(raw::LLVMTypeKind::LLVMFunctionTypeKind, "parameter_types", "a function type");
        // SAFETY: just checked to be a function type, and the buffer has room for all the parameters.
        unsafe {
            let count = raw::LLVMCountParamTypes(self.raw);
            let mut types = vec![std::ptr::null_mut(); count as usize];
            raw::LLVMGetParamTypes(self.raw, types.as_mut_ptr());
            types.into_iter().map(|parameter| Type::from_raw(parameter)).collect()
        }
    }

    // ---- Structs ----

    /// This struct's name, if it has one.
    ///
    /// # Panics
    /// If this isn't a struct (D3).
    #[must_use]
    #[track_caller]
    pub fn struct_name(self) -> Option<String> {
        self.require_kind(raw::LLVMTypeKind::LLVMStructTypeKind, "struct_name", "a struct");
        // SAFETY: just checked to be a struct; the name is copied before anything can change it.
        unsafe {
            let name = raw::LLVMGetStructName(self.raw);
            (!name.is_null()).then(|| from_borrowed(name))
        }
    }

    /// Gives this opaque named struct its body.
    ///
    /// # Panics
    /// If this isn't an opaque struct, or an element can't go in a struct (D3).
    #[track_caller]
    pub fn set_struct_body(self, elements: &[Type<'context>], is_packed: bool) {
        self.require_kind(raw::LLVMTypeKind::LLVMStructTypeKind, "set_struct_body", "a struct");
        // SAFETY: just checked to be a struct.
        if unsafe { raw::LLVMIsOpaqueStruct(self.raw) } == 0 {
            bad_call(format_args!("set_struct_body: {} already has a body", self.describe()));
        }
        check_struct_elements("set_struct_body", elements);
        // SAFETY: an opaque struct, with elements just checked; LLVM only reads them, and `Type` has the layout of
        // `LLVMTypeRef`.
        unsafe {
            raw::LLVMStructSetBody(
                self.raw,
                elements.as_ptr().cast_mut().cast(),
                length_as_u32(elements.len(), "set_struct_body"),
                i32::from(is_packed),
            );
        }
    }

    /// How many elements this struct has.
    ///
    /// # Panics
    /// If this isn't a struct (D3).
    #[must_use]
    #[track_caller]
    pub fn struct_element_count(self) -> u32 {
        self.require_kind(raw::LLVMTypeKind::LLVMStructTypeKind, "struct_element_count", "a struct");
        // SAFETY: just checked to be a struct.
        unsafe { raw::LLVMCountStructElementTypes(self.raw) }
    }

    /// This struct's element types.
    ///
    /// # Panics
    /// If this isn't a struct (D3).
    #[must_use]
    #[track_caller]
    pub fn struct_element_types(self) -> Vec<Type<'context>> {
        let count = self.struct_element_count();
        let mut types = vec![std::ptr::null_mut(); count as usize];
        // SAFETY: a struct, as `struct_element_count` checked, and the buffer has room for all its elements.
        unsafe { raw::LLVMGetStructElementTypes(self.raw, types.as_mut_ptr()) };
        // SAFETY: LLVM just wrote live types there.
        types.into_iter().map(|element| unsafe { Type::from_raw(element) }).collect()
    }

    /// This struct's element type at `index`.
    ///
    /// # Panics
    /// If this isn't a struct, or has no such element (D3).
    #[must_use]
    #[track_caller]
    pub fn struct_element_type_at(self, index: u32) -> Type<'context> {
        let count = self.struct_element_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("struct_element_type_at: index {index} is out of bounds for {}", self.describe()));
        }
        // SAFETY: a struct, and the index was just checked.
        unsafe { Type::from_raw(raw::LLVMStructGetTypeAtIndex(self.raw, index)) }
    }

    /// Whether this struct is packed.
    ///
    /// # Panics
    /// If this isn't a struct (D3).
    #[must_use]
    #[track_caller]
    pub fn is_packed_struct(self) -> bool {
        self.require_kind(raw::LLVMTypeKind::LLVMStructTypeKind, "is_packed_struct", "a struct");
        // SAFETY: just checked to be a struct.
        unsafe { raw::LLVMIsPackedStruct(self.raw) != 0 }
    }

    /// Whether this struct has no body yet.
    ///
    /// # Panics
    /// If this isn't a struct (D3).
    #[must_use]
    #[track_caller]
    pub fn is_opaque_struct(self) -> bool {
        self.require_kind(raw::LLVMTypeKind::LLVMStructTypeKind, "is_opaque_struct", "a struct");
        // SAFETY: just checked to be a struct.
        unsafe { raw::LLVMIsOpaqueStruct(self.raw) != 0 }
    }

    /// Whether this struct is a literal one, identified by its elements rather than a name.
    ///
    /// # Panics
    /// If this isn't a struct (D3).
    #[must_use]
    #[track_caller]
    pub fn is_literal_struct(self) -> bool {
        self.require_kind(raw::LLVMTypeKind::LLVMStructTypeKind, "is_literal_struct", "a struct");
        // SAFETY: just checked to be a struct.
        unsafe { raw::LLVMIsLiteralStruct(self.raw) != 0 }
    }

    // ---- Arrays and vectors ----

    /// The element type of this array or vector.
    ///
    /// # Panics
    /// If this is neither (D3).
    #[must_use]
    #[track_caller]
    pub fn element_type(self) -> Type<'context> {
        use raw::LLVMTypeKind as K;
        if !matches!(self.raw_kind(), K::LLVMArrayTypeKind | K::LLVMVectorTypeKind | K::LLVMScalableVectorTypeKind) {
            bad_call(format_args!("element_type: {} is not an array or a vector", self.describe()));
        }
        // SAFETY: just checked to be a type with an element type.
        unsafe { Type::from_raw(raw::LLVMGetElementType(self.raw)) }
    }

    /// How many elements this array has.
    ///
    /// # Panics
    /// If this isn't an array (D3).
    #[must_use]
    #[track_caller]
    pub fn array_length_in_elements(self) -> u64 {
        self.require_kind(raw::LLVMTypeKind::LLVMArrayTypeKind, "array_length_in_elements", "an array");
        // SAFETY: just checked to be an array.
        unsafe { raw::LLVMGetArrayLength2(self.raw) }
    }

    /// How many elements this vector has; for a scalable vector, how many per unit of its scale.
    ///
    /// # Panics
    /// If this isn't a vector (D3).
    #[must_use]
    #[track_caller]
    pub fn vector_size_in_elements(self) -> u32 {
        use raw::LLVMTypeKind as K;
        if !matches!(self.raw_kind(), K::LLVMVectorTypeKind | K::LLVMScalableVectorTypeKind) {
            bad_call(format_args!("vector_size_in_elements: {} is not a vector", self.describe()));
        }
        // SAFETY: just checked to be a vector.
        unsafe { raw::LLVMGetVectorSize(self.raw) }
    }

    /// An array of `length_in_elements` of this type.
    ///
    /// # Panics
    /// If this type can't be an array's element (D3).
    #[must_use]
    #[track_caller]
    pub fn array_type(self, length_in_elements: u64) -> Type<'context> {
        // SAFETY: only reads the type.
        if !unsafe { crate::shim::lyftar_is_valid_array_element(self.raw) } {
            bad_call(format_args!("array_type: {} can't be an array's element", self.describe()));
        }
        // SAFETY: just checked.
        unsafe { Type::from_raw(raw::LLVMArrayType2(self.raw, length_in_elements)) }
    }

    /// A fixed vector of `size_in_elements` of this type.
    ///
    /// # Panics
    /// If this type can't be a vector's element, or the size is zero (D3).
    #[must_use]
    #[track_caller]
    pub fn vector_type(self, size_in_elements: u32) -> Type<'context> {
        // SAFETY: only reads the type.
        if size_in_elements == 0 || !unsafe { crate::shim::lyftar_is_valid_vector_element(self.raw) } {
            bad_call(format_args!("vector_type: can't make a vector of {size_in_elements} {}", self.describe()));
        }
        // SAFETY: just checked.
        unsafe { Type::from_raw(raw::LLVMVectorType(self.raw, size_in_elements)) }
    }

    /// A scalable vector of `size_in_elements` of this type per unit of its scale.
    ///
    /// # Panics
    /// If this type can't be a vector's element, or the size is zero (D3).
    #[must_use]
    #[track_caller]
    pub fn scalable_vector_type(self, size_in_elements: u32) -> Type<'context> {
        // SAFETY: only reads the type.
        if size_in_elements == 0 || !unsafe { crate::shim::lyftar_is_valid_vector_element(self.raw) } {
            bad_call(format_args!(
                "scalable_vector_type: can't make a vector of {size_in_elements} {}",
                self.describe()
            ));
        }
        // SAFETY: just checked.
        unsafe { Type::from_raw(raw::LLVMScalableVectorType(self.raw, size_in_elements)) }
    }

    // ---- Target extension types ----

    /// This target extension type's name.
    ///
    /// # Panics
    /// If this isn't a target extension type (D3).
    #[must_use]
    #[track_caller]
    pub fn target_extension_name(self) -> String {
        self.require_kind(raw::LLVMTypeKind::LLVMTargetExtTypeKind, "target_extension_name", "a target extension type");
        // SAFETY: just checked.
        from_borrowed(unsafe { raw::LLVMGetTargetExtTypeName(self.raw) })
    }

    /// This target extension type's type parameters.
    ///
    /// # Panics
    /// If this isn't a target extension type (D3).
    #[must_use]
    #[track_caller]
    pub fn target_extension_type_parameters(self) -> Vec<Type<'context>> {
        self.require_kind(
            raw::LLVMTypeKind::LLVMTargetExtTypeKind,
            "target_extension_type_parameters",
            "a target extension type",
        );
        // SAFETY: just checked, and each index is below the count LLVM gave.
        unsafe {
            (0..raw::LLVMGetTargetExtTypeNumTypeParams(self.raw))
                .map(|index| Type::from_raw(raw::LLVMGetTargetExtTypeTypeParam(self.raw, index)))
                .collect()
        }
    }

    /// This target extension type's integer parameters.
    ///
    /// # Panics
    /// If this isn't a target extension type (D3).
    #[must_use]
    #[track_caller]
    pub fn target_extension_integer_parameters(self) -> Vec<u32> {
        self.require_kind(
            raw::LLVMTypeKind::LLVMTargetExtTypeKind,
            "target_extension_integer_parameters",
            "a target extension type",
        );
        // SAFETY: just checked, and each index is below the count LLVM gave.
        unsafe {
            (0..raw::LLVMGetTargetExtTypeNumIntParams(self.raw))
                .map(|index| raw::LLVMGetTargetExtTypeIntParam(self.raw, index))
                .collect()
        }
    }

    // ---- Constants of any type ----

    /// The zero value of this type (`zeroinitializer`, `null`, `0`).
    ///
    /// # Panics
    /// If this type has no zero value (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_null(self) -> Value<'context, 'static, T> {
        // SAFETY: only reads the type.
        if !unsafe { crate::shim::lyftar_type_has_null_value(self.raw) } {
            bad_call(format_args!("constant_null: {} has no zero value", self.describe()));
        }
        // SAFETY: just checked; the zero value of `T` is a `T`, and it belongs to the context.
        unsafe { Value::from_raw(raw::LLVMConstNull(self.raw)) }
    }

    /// `undef` of this type.
    ///
    /// # Panics
    /// If this type can't have an `undef` value (D3).
    #[must_use]
    #[track_caller]
    pub fn undefined(self) -> Value<'context, 'static, T> {
        self.require_value_type("undefined");
        // SAFETY: just checked; `undef` of `T` is a `T`, and it belongs to the context.
        unsafe { Value::from_raw(raw::LLVMGetUndef(self.raw)) }
    }

    /// `poison` of this type.
    ///
    /// # Panics
    /// If this type can't have a `poison` value (D3).
    #[must_use]
    #[track_caller]
    pub fn poison(self) -> Value<'context, 'static, T> {
        self.require_value_type("poison");
        // SAFETY: just checked; `poison` of `T` is a `T`, and it belongs to the context.
        unsafe { Value::from_raw(raw::LLVMGetPoison(self.raw)) }
    }

    #[track_caller]
    fn require_value_type(self, operation: &str) {
        use raw::LLVMTypeKind as K;
        if matches!(self.raw_kind(), K::LLVMVoidTypeKind | K::LLVMFunctionTypeKind | K::LLVMLabelTypeKind) {
            bad_call(format_args!("{operation}: {} can't be the type of a value", self.describe()));
        }
    }
}

impl Type<'_, Pointer> {
    /// This pointer's address space.
    #[must_use]
    pub fn address_space(self) -> u32 {
        // SAFETY: `Pointer` guarantees a pointer type.
        unsafe { raw::LLVMGetPointerAddressSpace(self.raw) }
    }
}

impl<W> Type<'_, Integer<W>> {
    /// This integer type's width.
    #[must_use]
    pub fn width_in_bits(self) -> u32 {
        // SAFETY: `Integer` guarantees an integer type.
        unsafe { raw::LLVMGetIntTypeWidth(self.raw) }
    }
}

// trace: D24, D30
// The marker guarantees a byte type, so none of these has to check for one when the line runs.
impl<'context, W> Type<'context, Byte<W>> {
    /// This byte type's width.
    #[must_use]
    pub fn width_in_bits(self) -> u32 {
        // SAFETY: `Byte` guarantees a byte type.
        unsafe { raw::LLVMGetByteTypeWidth(self.raw) }
    }

    /// A constant of this byte type.
    #[must_use]
    pub fn constant_byte(self, value: u64) -> Value<'context, 'static, Byte<W>> {
        // SAFETY: `Byte` guarantees a byte type; the constant belongs to the context.
        unsafe { Value::from_raw(raw::LLVMConstByte(self.raw, value)) }
    }

    /// A constant of this byte type wider than 64 bits, from little-endian 64-bit words.
    ///
    /// # Panics
    /// If there are no words (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_byte_from_words(self, words: &[u64]) -> Value<'context, 'static, Byte<W>> {
        if words.is_empty() {
            bad_call(format_args!("constant_byte_from_words: there must be at least one word"));
        }
        let count = length_as_u32(words.len(), "constant_byte_from_words");
        // SAFETY: `Byte` guarantees a byte type; LLVM reads `count` words.
        unsafe { Value::from_raw(raw::LLVMConstByteOfArbitraryPrecision(self.raw, count, words.as_ptr())) }
    }

    /// A constant of this byte type from text, in the given radix.
    ///
    /// # Panics
    /// If the radix isn't 2, 8, 10, 16 or 36, or `text` isn't a number in it (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_byte_from_string(self, text: &str, radix: u8) -> Value<'context, 'static, Byte<W>> {
        if !checks::integer_text_is_valid(text.as_bytes(), radix) {
            bad_call(format_args!("constant_byte_from_string: {text:?} is not a number in radix {radix}"));
        }
        // SAFETY: `Byte` guarantees a byte type, LLVM reads `text.len()` bytes, and the text was just checked.
        unsafe { Value::from_raw(raw::LLVMConstByteOfStringAndSize(self.raw, text.as_ptr().cast(), text.len(), radix)) }
    }
}

impl<'context> Context<'context> {
    /// The type `T`, when the Rust code spells it out.
    #[must_use]
    pub fn get_type<T: SpecificType>(self) -> Type<'context, T> {
        Type { raw: T::get_raw(self), marker: PhantomData }
    }

    /// An integer type whose width is only known when the line runs.
    ///
    /// # Panics
    /// If the width isn't between 1 and 2^23 bits (D3).
    #[must_use]
    #[track_caller]
    pub fn integer_type(self, width_in_bits: u32) -> Type<'context, Integer> {
        if !checks::integer_width_is_valid(width_in_bits) {
            bad_call(format_args!("integer_type: {width_in_bits} bits is not between 1 and 2^23 bits"));
        }
        // SAFETY: the width was just checked.
        Type { raw: unsafe { raw::LLVMIntTypeInContext(self.raw, width_in_bits) }, marker: PhantomData }
    }

    /// A byte type whose width is only known when the line runs. `get_type::<Byte<Bits<8>>>()` is for one the Rust code
    /// spells out.
    ///
    /// # Panics
    /// If the width isn't between 1 and 2^23 bits (D3).
    #[must_use]
    #[track_caller]
    pub fn byte_type(self, width_in_bits: u32) -> Type<'context, Byte> {
        if !checks::integer_width_is_valid(width_in_bits) {
            bad_call(format_args!("byte_type: {width_in_bits} bits is not between 1 and 2^23 bits"));
        }
        // SAFETY: the width was just checked.
        Type { raw: unsafe { raw::LLVMByteTypeInContext(self.raw, width_in_bits) }, marker: PhantomData }
    }

    /// `void`.
    #[must_use]
    pub fn void_type(self) -> Type<'context> {
        // SAFETY: the context is live.
        Type { raw: unsafe { raw::LLVMVoidTypeInContext(self.raw) }, marker: PhantomData }
    }

    /// `label`.
    #[must_use]
    pub fn label_type(self) -> Type<'context> {
        // SAFETY: the context is live.
        Type { raw: unsafe { raw::LLVMLabelTypeInContext(self.raw) }, marker: PhantomData }
    }

    /// `metadata`.
    #[must_use]
    pub fn metadata_type(self) -> Type<'context> {
        // SAFETY: the context is live.
        Type { raw: unsafe { raw::LLVMMetadataTypeInContext(self.raw) }, marker: PhantomData }
    }

    /// `token`.
    #[must_use]
    pub fn token_type(self) -> Type<'context> {
        // SAFETY: the context is live.
        Type { raw: unsafe { raw::LLVMTokenTypeInContext(self.raw) }, marker: PhantomData }
    }

    /// `x86_amx`.
    #[must_use]
    pub fn x86_advanced_matrix_extensions_type(self) -> Type<'context> {
        // SAFETY: the context is live.
        Type { raw: unsafe { raw::LLVMX86AMXTypeInContext(self.raw) }, marker: PhantomData }
    }

    /// A pointer in the given address space.
    #[must_use]
    pub fn pointer_type(self, address_space: u32) -> Type<'context, Pointer> {
        // SAFETY: the context is live.
        Type { raw: unsafe { raw::LLVMPointerTypeInContext(self.raw, address_space) }, marker: PhantomData }
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
        // SAFETY: only reads the types, and `Type` has the layout of `LLVMTypeRef`.
        let bad_parameter = unsafe {
            crate::shim::lyftar_first_invalid_function_type_part(
                return_type.raw,
                parameter_types.as_ptr().cast(),
                length_as_u32(parameter_types.len(), "function_type"),
            )
        };
        if bad_parameter != NO_INVALID_PART {
            bad_call(format_args!(
                "function_type: {} can't be used in a function",
                if bad_parameter as usize >= parameter_types.len() {
                    return_type.describe()
                } else {
                    parameter_types[bad_parameter as usize].describe()
                }
            ));
        }
        // SAFETY: LLVM only reads the parameter types, which were just checked.
        let raw = unsafe {
            raw::LLVMFunctionType(
                return_type.raw,
                parameter_types.as_ptr().cast_mut().cast(),
                length_as_u32(parameter_types.len(), "function_type"),
                i32::from(is_variadic),
            )
        };
        Type { raw, marker: PhantomData }
    }

    /// A literal struct.
    ///
    /// # Panics
    /// If an element can't go in a struct (D3).
    #[must_use]
    #[track_caller]
    pub fn struct_type(self, elements: &[Type<'context>], is_packed: bool) -> Type<'context> {
        check_struct_elements("struct_type", elements);
        // SAFETY: LLVM only reads the elements, which were just checked.
        let raw = unsafe {
            raw::LLVMStructTypeInContext(
                self.raw,
                elements.as_ptr().cast_mut().cast(),
                length_as_u32(elements.len(), "struct_type"),
                i32::from(is_packed),
            )
        };
        Type { raw, marker: PhantomData }
    }

    /// A new opaque named struct, whose body [`Type::set_struct_body`] gives later.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn create_named_struct(self, name: &str) -> Type<'context> {
        let name = c_string(name, "create_named_struct");
        // SAFETY: the name is a valid C string for the call.
        Type { raw: unsafe { raw::LLVMStructCreateNamed(self.raw, name.as_ptr()) }, marker: PhantomData }
    }

    /// The named struct called `name` in this context, if there is one.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn get_named_struct(self, name: &str) -> Option<Type<'context>> {
        let name = c_string(name, "get_named_struct");
        // SAFETY: the name is a valid C string for the call.
        let raw = unsafe { raw::LLVMGetTypeByName2(self.raw, name.as_ptr()) };
        (!raw.is_null()).then_some(Type { raw, marker: PhantomData })
    }

    /// A target extension type.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn target_extension_type(
        self,
        name: &str,
        type_parameters: &[Type<'context>],
        integer_parameters: &[u32],
    ) -> Type<'context> {
        let name = c_string(name, "target_extension_type");
        // SAFETY: the shim only reads them, and `Type` has the layout of `LLVMTypeRef`.
        let valid = unsafe {
            crate::shim::lyftar_target_extension_type_is_valid(
                self.raw,
                name.as_ptr(),
                type_parameters.as_ptr().cast(),
                length_as_u32(type_parameters.len(), "target_extension_type"),
                integer_parameters.as_ptr(),
                length_as_u32(integer_parameters.len(), "target_extension_type"),
            )
        };
        if !valid {
            bad_call(format_args!(
                "target_extension_type: LLVM has rules for {name:?}, and these parameters break them"
            ));
        }
        // SAFETY: just checked; LLVM only reads the parameters, whose counts are passed with them.
        let raw = unsafe {
            raw::LLVMTargetExtTypeInContext(
                self.raw,
                name.as_ptr(),
                type_parameters.as_ptr().cast_mut().cast(),
                length_as_u32(type_parameters.len(), "target_extension_type"),
                integer_parameters.as_ptr().cast_mut().cast::<c_uint>(),
                length_as_u32(integer_parameters.len(), "target_extension_type"),
            )
        };
        Type { raw, marker: PhantomData }
    }
}

/// What the shim gives back when every part of a function type is valid.
const NO_INVALID_PART: u32 = u32::MAX;

#[track_caller]
fn check_struct_elements(operation: &str, elements: &[Type<'_>]) {
    for element in elements {
        // SAFETY: only reads the type.
        if !unsafe { crate::shim::lyftar_is_valid_struct_element(element.raw) } {
            bad_call(format_args!("{operation}: {} can't go in a struct", element.describe()));
        }
    }
}

/// A length as the `unsigned` LLVM wants. More than 4 billion of anything is a bug in the compiler using Lyftar.
#[track_caller]
pub(crate) fn length_as_u32(length: usize, operation: &str) -> u32 {
    match u32::try_from(length) {
        Ok(converted) => converted,
        Err(_) => bad_call(format_args!("{operation}: {length} is more than LLVM can take")),
    }
}
