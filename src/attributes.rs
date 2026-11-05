//! Attributes on functions, parameters, return values and calls.

use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::context::Context;
use crate::raw;
use crate::shim;
use crate::text::from_pointer_and_length;
use crate::types::{Invariant, Type, length_as_u32};

/// An attribute, which belongs to the context.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Attribute<'context> {
    pub(crate) raw: raw::LLVMAttributeRef,
    marker: Invariant<'context>,
}

/// An LLVM attribute's kind, from [`AttributeKind::for_name`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AttributeKind(pub(crate) u32);

/// How a target-independent floating-point environment treats denormal numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DenormalMode {
    /// As IEEE 754 says.
    Ieee754,
    /// Flushed to zero, keeping the sign.
    PreserveSign,
    /// Flushed to positive zero.
    PositiveZero,
    /// Decided at run time.
    Dynamic,
}

impl DenormalMode {
    fn to_raw(self) -> raw::LLVMDenormalModeKind {
        match self {
            DenormalMode::Ieee754 => raw::LLVMDenormalModeKind::LLVMDenormalModeKindIEEE,
            DenormalMode::PreserveSign => raw::LLVMDenormalModeKind::LLVMDenormalModeKindPreserveSign,
            DenormalMode::PositiveZero => raw::LLVMDenormalModeKind::LLVMDenormalModeKindPositiveZero,
            DenormalMode::Dynamic => raw::LLVMDenormalModeKind::LLVMDenormalModeKindDynamic,
        }
    }
}

impl AttributeKind {
    /// The kind LLVM calls `name`, such as `nounwind`, if there is one.
    #[must_use]
    pub fn for_name(name: &str) -> Option<AttributeKind> {
        // SAFETY: LLVM reads `name.len()` bytes.
        let kind = unsafe { raw::LLVMGetEnumAttributeKindForName(name.as_ptr().cast(), name.len()) };
        (kind != 0).then_some(AttributeKind(kind))
    }

    /// The highest kind number LLVM has.
    #[must_use]
    pub fn last_kind() -> u32 {
        // SAFETY: takes nothing.
        unsafe { raw::LLVMGetLastEnumAttributeKind() }
    }
}

/// Where an attribute goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeLocation {
    /// On the return value.
    Return,
    /// On the function as a whole.
    Function,
    /// On the parameter at this index, from zero.
    Parameter(u32),
}

impl AttributeLocation {
    pub(crate) fn to_raw(self) -> raw::LLVMAttributeIndex {
        match self {
            AttributeLocation::Return => 0,
            // LLVM declares this one as -1 in an enum of `unsigned`, so all ones.
            AttributeLocation::Function => u32::MAX,
            // LLVM's parameter indices start at 1.
            AttributeLocation::Parameter(index) => index.saturating_add(1),
        }
    }
}

impl<'context> Attribute<'context> {
    /// Wraps a raw attribute.
    ///
    /// # Safety
    /// `raw` must be a live attribute of this context.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMAttributeRef) -> Self {
        Attribute { raw, marker: PhantomData }
    }

    /// The raw attribute.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(self) -> raw::LLVMAttributeRef {
        self.raw
    }

    /// Whether this is an enum attribute, which holds a kind and maybe a number.
    #[must_use]
    pub fn is_enum(self) -> bool {
        // SAFETY: the attribute is live for `'context`.
        unsafe { raw::LLVMIsEnumAttribute(self.raw) != 0 }
    }

    /// Whether this is a string attribute.
    #[must_use]
    pub fn is_string(self) -> bool {
        // SAFETY: the attribute is live for `'context`.
        unsafe { raw::LLVMIsStringAttribute(self.raw) != 0 }
    }

    /// Whether this is a type attribute.
    #[must_use]
    pub fn is_type(self) -> bool {
        // SAFETY: the attribute is live for `'context`.
        unsafe { raw::LLVMIsTypeAttribute(self.raw) != 0 }
    }

    /// This attribute's kind, if it is an enum or type attribute.
    #[must_use]
    pub fn kind(self) -> Option<AttributeKind> {
        // SAFETY: just checked to be a class LLVM takes here.
        (!self.is_string()).then(|| AttributeKind(unsafe { raw::LLVMGetEnumAttributeKind(self.raw) }))
    }

    /// This attribute's number, if it is an enum attribute.
    #[must_use]
    pub fn value(self) -> Option<u64> {
        // SAFETY: just checked to be a class LLVM takes here.
        self.is_enum().then(|| unsafe { raw::LLVMGetEnumAttributeValue(self.raw) })
    }

    /// This attribute's type, if it is a type attribute.
    #[must_use]
    pub fn type_value(self) -> Option<Type<'context>> {
        // SAFETY: just checked to be a class LLVM takes here, and the type is live for `'context`.
        self.is_type().then(|| unsafe { Type::from_raw(raw::LLVMGetTypeAttributeValue(self.raw)) })
    }

    /// This attribute's key, if it is a string attribute.
    #[must_use]
    pub fn string_key(self) -> Option<String> {
        if !self.is_string() {
            return None;
        }
        let mut length = 0;
        // SAFETY: just checked to be a string attribute; the text is copied right away.
        let text = unsafe { raw::LLVMGetStringAttributeKind(self.raw, &raw mut length) };
        Some(from_pointer_and_length(text, length as usize))
    }

    /// This attribute's text value, if it is a string attribute.
    #[must_use]
    pub fn string_value(self) -> Option<String> {
        if !self.is_string() {
            return None;
        }
        let mut length = 0;
        // SAFETY: just checked to be a string attribute; the text is copied right away.
        let text = unsafe { raw::LLVMGetStringAttributeValue(self.raw, &raw mut length) };
        Some(from_pointer_and_length(text, length as usize))
    }
}

impl std::fmt::Debug for Attribute<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Attribute({:?}, {:?}, {:?})", self.kind(), self.value(), self.string_key())
    }
}

impl<'context> Context<'context> {
    /// An enum attribute, with a number for the kinds that take one, such as `align` or `dereferenceable`.
    ///
    /// # Panics
    /// If this kind doesn't hold a plain number or nothing, such as a type attribute's kind (D3).
    #[must_use]
    #[track_caller]
    pub fn enum_attribute(self, kind: AttributeKind, value: u64) -> Attribute<'context> {
        // SAFETY: takes a number and looks it up.
        let allowed = unsafe { shim::lyftar_attribute_kind_takes_integer_or_nothing(kind.0, value) };
        if !allowed {
            bad_call(format_args!("enum_attribute: kind {} can't hold the number {value}", kind.0));
        }
        // SAFETY: the kind was just checked.
        Attribute { raw: unsafe { raw::LLVMCreateEnumAttribute(self.raw, kind.0, value) }, marker: PhantomData }
    }

    /// A type attribute, such as `sret` or `byval`.
    ///
    /// # Panics
    /// If this kind doesn't hold a type (D3).
    #[must_use]
    #[track_caller]
    pub fn type_attribute(self, kind: AttributeKind, attribute_type: Type<'context>) -> Attribute<'context> {
        // SAFETY: takes a number and looks it up.
        if !unsafe { shim::lyftar_attribute_kind_takes_type(kind.0) } {
            bad_call(format_args!("type_attribute: kind {} doesn't hold a type", kind.0));
        }
        // SAFETY: the kind was just checked, and the type is live for `'context`.
        Attribute {
            raw: unsafe { raw::LLVMCreateTypeAttribute(self.raw, kind.0, attribute_type.raw) },
            marker: PhantomData,
        }
    }

    /// A constant-range attribute such as `range`, whose words are little-endian.
    ///
    /// # Panics
    /// If this kind doesn't hold a range, or there aren't enough words for `width_in_bits`, or the range is one LLVM's
    /// `ConstantRange` can't hold (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_range_attribute(
        self,
        kind: AttributeKind,
        width_in_bits: u32,
        lower_words: &[u64],
        upper_words: &[u64],
    ) -> Attribute<'context> {
        // SAFETY: takes a number and looks it up.
        if !unsafe { shim::lyftar_attribute_kind_takes_constant_range(kind.0) } {
            bad_call(format_args!("constant_range_attribute: kind {} doesn't hold a range", kind.0));
        }
        let needed = (width_in_bits as usize).div_ceil(64);
        if width_in_bits == 0 || lower_words.len() < needed || upper_words.len() < needed {
            bad_call(format_args!("constant_range_attribute: {width_in_bits} bits needs {needed} words at each end"));
        }
        // SAFETY: the words were just counted; the shim only reads them.
        let valid =
            unsafe { shim::lyftar_constant_range_is_valid(width_in_bits, lower_words.as_ptr(), upper_words.as_ptr()) };
        if !valid {
            bad_call(format_args!(
                "constant_range_attribute: lower and upper are equal, and neither all-zeros nor all-ones"
            ));
        }
        // SAFETY: the kind and words were just checked; LLVM reads `needed` words at each end.
        Attribute {
            raw: unsafe {
                raw::LLVMCreateConstantRangeAttribute(
                    self.raw,
                    kind.0,
                    width_in_bits,
                    lower_words.as_ptr(),
                    upper_words.as_ptr(),
                )
            },
            marker: PhantomData,
        }
    }

    /// A `denormal_fpenv` attribute: output and input modes for the default environment, and for `float`.
    #[must_use]
    pub fn denormal_float_environment_attribute(
        self,
        default_output: DenormalMode,
        default_input: DenormalMode,
        float_output: DenormalMode,
        float_input: DenormalMode,
    ) -> Attribute<'context> {
        // SAFETY: the context is live, and the modes are all valid enumerators.
        Attribute {
            raw: unsafe {
                raw::LLVMCreateDenormalFPEnvAttribute(
                    self.raw,
                    default_output.to_raw(),
                    default_input.to_raw(),
                    float_output.to_raw(),
                    float_input.to_raw(),
                )
            },
            marker: PhantomData,
        }
    }

    /// A string attribute: a key and a value.
    #[must_use]
    #[track_caller]
    pub fn string_attribute(self, key: &str, value: &str) -> Attribute<'context> {
        let key_length = length_as_u32(key.len(), "string_attribute");
        let value_length = length_as_u32(value.len(), "string_attribute");
        // SAFETY: LLVM reads the lengths given.
        Attribute {
            raw: unsafe {
                raw::LLVMCreateStringAttribute(
                    self.raw,
                    key.as_ptr().cast(),
                    key_length,
                    value.as_ptr().cast(),
                    value_length,
                )
            },
            marker: PhantomData,
        }
    }
}
