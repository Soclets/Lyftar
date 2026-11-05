//! Constants. The ones made only from the context carry `'static` for their module borrow, so they can be used with any
//! module of the context, and survive deletes and pass runs (D11). A constant made from a global borrows its module.

use crate::bad_call::bad_call;
use crate::block::Block;
use crate::checks;
use crate::context::Context;
use crate::enums::{InlineAssemblyDialect, Opcode};
use crate::raw;
use crate::shim;
use crate::text::from_pointer_and_length;
use crate::types::{Dynamic, Float, Integer, Pointer, Type, length_as_u32};
use crate::values::{
    BlockAddress, ConstantDataSequential, ConstantExpression, ConstantFloat, ConstantInteger, ConstantMarker,
    ConstantPointerAuthentication, Function, InlineAssembly, Value,
};

impl<'context, W> Type<'context, Integer<W>> {
    /// An integer constant, cut to this type's width like LLVM's `ConstantInt`.
    #[must_use]
    pub fn constant_integer(self, value: u64, sign_extend: bool) -> Value<'context, 'static, Integer<W>> {
        // SAFETY: `Integer` guarantees an integer type; the constant belongs to the context.
        unsafe { Value::from_raw(raw::LLVMConstInt(self.raw, value, i32::from(sign_extend))) }
    }

    /// An integer constant wider than 64 bits, from little-endian 64-bit words.
    #[must_use]
    #[track_caller]
    pub fn constant_integer_from_words(self, words: &[u64]) -> Value<'context, 'static, Integer<W>> {
        if words.is_empty() {
            bad_call(format_args!("constant_integer_from_words: there must be at least one word"));
        }
        let count = length_as_u32(words.len(), "constant_integer_from_words");
        // SAFETY: `Integer` guarantees an integer type; LLVM reads `count` words.
        unsafe { Value::from_raw(raw::LLVMConstIntOfArbitraryPrecision(self.raw, count, words.as_ptr())) }
    }

    /// An integer constant from text, in the given radix.
    ///
    /// # Panics
    /// If the radix isn't 2, 8, 10, 16 or 36, or `text` isn't a number in it (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_integer_from_string(self, text: &str, radix: u8) -> Value<'context, 'static, Integer<W>> {
        if !checks::integer_text_is_valid(text.as_bytes(), radix) {
            bad_call(format_args!("constant_integer_from_string: {text:?} is not a number in radix {radix}"));
        }
        let length = length_as_u32(text.len(), "constant_integer_from_string");
        // SAFETY: `Integer` guarantees an integer type, LLVM reads `length` bytes, and the text was just checked.
        unsafe { Value::from_raw(raw::LLVMConstIntOfStringAndSize(self.raw, text.as_ptr().cast(), length, radix)) }
    }

    /// The constant with every bit set.
    #[must_use]
    pub fn constant_all_ones(self) -> Value<'context, 'static, Integer<W>> {
        // SAFETY: `Integer` guarantees an integer type, which LLVM takes here.
        unsafe { Value::from_raw(raw::LLVMConstAllOnes(self.raw)) }
    }
}

impl<'context, F> Type<'context, Float<F>> {
    /// A floating-point constant, the nearest one of this format to `value`.
    #[must_use]
    pub fn constant_float(self, value: f64) -> Value<'context, 'static, Float<F>> {
        // SAFETY: `Float` guarantees a floating-point type; the constant belongs to the context.
        unsafe { Value::from_raw(raw::LLVMConstReal(self.raw, value)) }
    }

    /// A floating-point constant from text, as in `"1.5"`.
    ///
    /// # Panics
    /// If `text` isn't a number (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_float_from_string(self, text: &str) -> Value<'context, 'static, Float<F>> {
        // `LLVMConstRealOfStringAndSize` aborts on text that isn't a number, so it is parsed here first. Rust's float
        // grammar is narrower than LLVM's, and everything it accepts LLVM accepts.
        if text.parse::<f64>().is_err() {
            bad_call(format_args!("constant_float_from_string: {text:?} is not a number"));
        }
        let length = length_as_u32(text.len(), "constant_float_from_string");
        // SAFETY: `Float` guarantees a floating-point type, LLVM reads `length` bytes, and the text was just checked.
        unsafe { Value::from_raw(raw::LLVMConstRealOfStringAndSize(self.raw, text.as_ptr().cast(), length)) }
    }

    /// A floating-point constant from its bits, as little-endian 64-bit words.
    ///
    /// # Panics
    /// If there aren't enough words for this format (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_float_from_bits(self, words: &[u64]) -> Value<'context, 'static, Float<F>> {
        // SAFETY: only reads the type.
        let needed = unsafe { shim::lyftar_float_size_in_words(self.raw) };
        if !checks::enough_words(words.len(), needed) {
            bad_call(format_args!(
                "constant_float_from_bits: {} needs {needed} words, not {}",
                self.print_to_string(),
                words.len()
            ));
        }
        // SAFETY: `Float` guarantees a floating-point type, and there are enough words for LLVM to read.
        unsafe { Value::from_raw(raw::LLVMConstFPFromBits(self.raw, words.as_ptr())) }
    }
}

impl<'context, T> Type<'context, T> {
    /// A constant array of this element type.
    ///
    /// # Panics
    /// If a value has another type, or isn't a constant (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_array<'module, V>(self, values: &[Value<'context, 'module, V>]) -> Value<'context, 'module> {
        // SAFETY: only reads the type.
        if !unsafe { shim::lyftar_is_valid_array_element(self.raw) } {
            bad_call(format_args!("constant_array: {} can't be an array's element", self.print_to_string()));
        }
        check_constants("constant_array", values, Some(self.raw));
        // SAFETY: LLVM only reads the values, which were just checked; `Value` has the layout of `LLVMValueRef`.
        unsafe {
            Value::from_raw(raw::LLVMConstArray2(self.raw, values.as_ptr().cast_mut().cast(), values.len() as u64))
        }
    }

    /// A constant array of this element type from raw data, `data.len()` bytes of it.
    ///
    /// # Panics
    /// If this type's elements don't fill the data exactly, or it can't be stored as plain data (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_data_array(self, data: &[u8]) -> Value<'context, 'static> {
        // SAFETY: only reads the type.
        let size_in_bits = unsafe { shim::lyftar_data_element_size_in_bits(self.raw) };
        if !checks::data_fits_elements(data.len() as u64, size_in_bits) {
            bad_call(format_args!(
                "constant_data_array: {} bytes can't be elements of type {}",
                data.len(),
                self.print_to_string()
            ));
        }
        // SAFETY: LLVM reads `data.len()` bytes, and the element type was just checked.
        unsafe { Value::from_raw(raw::LLVMConstDataArray(self.raw, data.as_ptr().cast(), data.len())) }
    }

    /// A constant of this named struct type.
    ///
    /// # Panics
    /// If this isn't a struct with that many elements of those types, or a value isn't a constant (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_named_struct<'module, V>(self, values: &[Value<'context, 'module, V>]) -> Value<'context, 'module> {
        let elements = self.struct_element_types();
        let count = length_as_u32(values.len(), "constant_named_struct");
        if elements.len() != values.len() {
            bad_call(format_args!(
                "constant_named_struct: {} has {} elements, not {count}",
                self.print_to_string(),
                elements.len()
            ));
        }
        check_constants("constant_named_struct", values, None);
        for (element, value) in elements.iter().zip(values) {
            // SAFETY: the value is live.
            if !checks::same_identity(element.raw as usize, unsafe { raw::LLVMTypeOf(value.raw) } as usize) {
                bad_call(format_args!(
                    "constant_named_struct: {} is not a {}",
                    value.print_to_string(),
                    element.print_to_string()
                ));
            }
        }
        // SAFETY: LLVM only reads the values, which were just checked.
        unsafe { Value::from_raw(raw::LLVMConstNamedStruct(self.raw, values.as_ptr().cast_mut().cast(), count)) }
    }

    /// Inline assembly of this function type.
    ///
    /// # Panics
    /// If this isn't a function type, or LLVM rejects the constraints for it (D3).
    #[must_use]
    #[track_caller]
    #[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
    pub fn inline_assembly(
        self,
        assembly: &str,
        constraints: &str,
        has_side_effects: bool,
        needs_aligned_stack: bool,
        dialect: InlineAssemblyDialect,
        can_unwind: bool,
    ) -> Value<'context, 'static, InlineAssembly> {
        // SAFETY: only reads the type and the text.
        let valid = unsafe {
            shim::lyftar_inline_assembly_constraints_are_valid(self.raw, constraints.as_ptr().cast(), constraints.len())
        };
        if !valid {
            bad_call(format_args!(
                "inline_assembly: {constraints:?} are not valid constraints for {}",
                self.print_to_string()
            ));
        }
        // SAFETY: the type and constraints were just checked; LLVM reads the lengths given.
        unsafe {
            Value::from_raw(raw::LLVMGetInlineAsm(
                self.raw,
                assembly.as_ptr().cast(),
                assembly.len(),
                constraints.as_ptr().cast(),
                constraints.len(),
                i32::from(has_side_effects),
                i32::from(needs_aligned_stack),
                dialect.to_raw(),
                i32::from(can_unwind),
            ))
        }
    }
}

impl<'context> Context<'context> {
    /// A constant `[N x i8]` holding `bytes`, with a NUL at the end if `null_terminate` says so.
    #[must_use]
    pub fn constant_string(self, bytes: &[u8], null_terminate: bool) -> Value<'context, 'static> {
        // SAFETY: LLVM reads `bytes.len()` bytes; the constant belongs to the context.
        unsafe {
            Value::from_raw(raw::LLVMConstStringInContext2(
                self.raw,
                bytes.as_ptr().cast(),
                bytes.len(),
                i32::from(!null_terminate),
            ))
        }
    }

    /// A constant literal struct.
    ///
    /// # Panics
    /// If a value isn't a constant (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_struct<'module, V>(
        self,
        values: &[Value<'context, 'module, V>],
        is_packed: bool,
    ) -> Value<'context, 'module> {
        check_constants("constant_struct", values, None);
        for value in values {
            // SAFETY: the value is live, and the shim only reads its type.
            if !unsafe { shim::lyftar_is_valid_struct_element(raw::LLVMTypeOf(value.raw)) } {
                bad_call(format_args!("constant_struct: {} can't go in a struct", value.print_to_string()));
            }
        }
        let count = length_as_u32(values.len(), "constant_struct");
        // SAFETY: LLVM only reads the values, which were just checked.
        unsafe {
            Value::from_raw(raw::LLVMConstStructInContext(
                self.raw,
                values.as_ptr().cast_mut().cast(),
                count,
                i32::from(is_packed),
            ))
        }
    }

    /// A constant vector.
    ///
    /// # Panics
    /// If there are no values, or they have different types, or one isn't a constant (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_vector<'module, V>(self, values: &[Value<'context, 'module, V>]) -> Value<'context, 'module> {
        let Some(first) = values.first() else {
            bad_call(format_args!("constant_vector: a vector has at least one element"))
        };
        // SAFETY: the value is live.
        check_constants("constant_vector", values, Some(unsafe { raw::LLVMTypeOf(first.raw) }));
        // SAFETY: the value is live, and the shim only reads its type.
        if !unsafe { shim::lyftar_is_valid_vector_element(raw::LLVMTypeOf(first.raw)) } {
            bad_call(format_args!("constant_vector: {} can't be a vector's element", first.print_to_string()));
        }
        let count = length_as_u32(values.len(), "constant_vector");
        // SAFETY: LLVM only reads the values, which were just checked.
        unsafe { Value::from_raw(raw::LLVMConstVector(values.as_ptr().cast_mut().cast(), count)) }
    }
}

// trace: D3
#[track_caller]
fn check_constants<V>(operation: &str, values: &[Value<'_, '_, V>], element_type: Option<raw::LLVMTypeRef>) {
    for value in values {
        if !value.is_constant() {
            bad_call(format_args!("{operation}: {} is not a constant", value.print_to_string()));
        }
        if let Some(wanted) = element_type {
            // SAFETY: the value is live.
            if !checks::same_identity(wanted as usize, unsafe { raw::LLVMTypeOf(value.raw) } as usize) {
                // SAFETY: the type is live for as long as the value's context.
                let wanted_type: Type<'_, Dynamic> = unsafe { Type::from_raw(wanted) };
                bad_call(format_args!(
                    "{operation}: {} is not a {}",
                    value.print_to_string(),
                    wanted_type.print_to_string()
                ));
            }
        }
    }
}

impl Value<'_, '_, ConstantInteger> {
    /// This constant, zero-extended, if it fits in 64 bits.
    #[must_use]
    pub fn zero_extended_value(self) -> Option<u64> {
        // SAFETY: the marker guarantees a constant integer, and the value is only asked for when it fits.
        unsafe {
            shim::lyftar_constant_integer_fits_unsigned(self.raw).then(|| raw::LLVMConstIntGetZExtValue(self.raw))
        }
    }

    /// This constant, sign-extended, if it fits in 64 bits.
    #[must_use]
    pub fn sign_extended_value(self) -> Option<i64> {
        // SAFETY: the marker guarantees a constant integer, and the value is only asked for when it fits.
        unsafe { shim::lyftar_constant_integer_fits_signed(self.raw).then(|| raw::LLVMConstIntGetSExtValue(self.raw)) }
    }

    /// This constant's width.
    #[must_use]
    pub fn width_in_bits(self) -> u32 {
        // SAFETY: a constant integer has an integer type.
        unsafe { raw::LLVMGetIntTypeWidth(raw::LLVMTypeOf(self.raw)) }
    }
}

impl Value<'_, '_, ConstantFloat> {
    /// This constant as a double, and whether that lost information.
    #[must_use]
    pub fn as_double(self) -> (f64, bool) {
        let mut loses_information = 0;
        // SAFETY: the marker guarantees a constant float, and LLVM writes one flag.
        let value = unsafe { raw::LLVMConstRealGetDouble(self.raw, &raw mut loses_information) };
        (value, loses_information != 0)
    }
}

impl<'context, 'module> Value<'context, 'module, ConstantDataSequential> {
    /// Whether this constant is a string: an array of `i8`.
    #[must_use]
    pub fn is_constant_string(self) -> bool {
        // SAFETY: the marker guarantees what LLVM asks for.
        unsafe { raw::LLVMIsConstantString(self.raw) != 0 }
    }

    /// This constant's text, if it is an array of `i8`, with any NUL at its end left off.
    #[must_use]
    pub fn as_string(self) -> Option<Vec<u8>> {
        if !self.is_constant_string() {
            return None;
        }
        let mut length = 0;
        // SAFETY: just checked that it is a string; the bytes are copied right away.
        let text = unsafe { raw::LLVMGetAsString(self.raw, &raw mut length) };
        // SAFETY: LLVM says where the bytes are and how many.
        Some(unsafe { std::slice::from_raw_parts(text.cast::<u8>(), length) }.to_vec())
    }

    /// This constant's bytes as written.
    #[must_use]
    pub fn raw_data(self) -> Vec<u8> {
        let mut size_in_bytes = 0;
        // SAFETY: the marker guarantees what LLVM asks for; the bytes are copied right away.
        unsafe {
            let data = raw::LLVMGetRawDataValues(self.raw, &raw mut size_in_bytes);
            std::slice::from_raw_parts(data.cast::<u8>(), size_in_bytes).to_vec()
        }
    }

    /// The element at `index`.
    ///
    /// # Panics
    /// If there is no such element (D3).
    #[must_use]
    #[track_caller]
    pub fn element(self, index: u32) -> Value<'context, 'module> {
        // SAFETY: the marker guarantees what LLVM asks for, and the shim only reads it.
        let count = unsafe { shim::lyftar_constant_data_element_count(self.raw) };
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("element: index {index} is out of bounds for {count} elements"));
        }
        // SAFETY: the index was just checked.
        unsafe { Value::from_raw(raw::LLVMGetElementAsConstant(self.raw, index)) }
    }
}

impl Value<'_, '_, InlineAssembly> {
    /// The assembly text.
    #[must_use]
    pub fn assembly(self) -> String {
        let mut length = 0;
        // SAFETY: the marker guarantees inline assembly; the text is copied right away.
        let text = unsafe { raw::LLVMGetInlineAsmAsmString(self.raw, &raw mut length) };
        from_pointer_and_length(text, length)
    }

    /// The constraint text.
    #[must_use]
    pub fn constraints(self) -> String {
        let mut length = 0;
        // SAFETY: the marker guarantees inline assembly; the text is copied right away.
        let text = unsafe { raw::LLVMGetInlineAsmConstraintString(self.raw, &raw mut length) };
        from_pointer_and_length(text, length)
    }

    /// The syntax it is written in.
    #[must_use]
    pub fn dialect(self) -> InlineAssemblyDialect {
        // SAFETY: the marker guarantees inline assembly.
        InlineAssemblyDialect::from_raw(unsafe { raw::LLVMGetInlineAsmDialect(self.raw) })
    }

    /// Whether it has side effects.
    #[must_use]
    pub fn has_side_effects(self) -> bool {
        // SAFETY: the marker guarantees inline assembly.
        unsafe { raw::LLVMGetInlineAsmHasSideEffects(self.raw) != 0 }
    }

    /// Whether it needs an aligned stack.
    #[must_use]
    pub fn needs_aligned_stack(self) -> bool {
        // SAFETY: the marker guarantees inline assembly.
        unsafe { raw::LLVMGetInlineAsmNeedsAlignedStack(self.raw) != 0 }
    }

    /// Whether it can unwind.
    #[must_use]
    pub fn can_unwind(self) -> bool {
        // SAFETY: the marker guarantees inline assembly.
        unsafe { raw::LLVMGetInlineAsmCanUnwind(self.raw) != 0 }
    }
}

impl<'context> Value<'context, '_, InlineAssembly> {
    /// Its function type.
    #[must_use]
    pub fn function_type(self) -> Type<'context> {
        // SAFETY: the marker guarantees inline assembly.
        unsafe { Type::from_raw(raw::LLVMGetInlineAsmFunctionType(self.raw)) }
    }
}

impl<'context, 'module, T: ConstantMarker> Value<'context, 'module, T> {
    /// The element of this aggregate at `index`, if there is one.
    #[must_use]
    pub fn aggregate_element(self, index: u32) -> Option<Value<'context, 'module>> {
        // SAFETY: the marker guarantees a constant, and LLVM gives back null when there is no such element.
        let element = unsafe { raw::LLVMGetAggregateElement(self.raw, index) };
        // SAFETY: LLVM gave back a live constant.
        (!element.is_null()).then(|| unsafe { Value::from_raw(element) })
    }
}

impl<'context, 'module> Value<'context, 'module, ConstantExpression> {
    /// This constant expression's operation.
    #[must_use]
    pub fn constant_opcode(self) -> Opcode {
        // SAFETY: the marker guarantees a constant expression.
        Opcode::from_raw(unsafe { raw::LLVMGetConstOpcode(self.raw) })
    }
}

impl<'context, 'module> Value<'context, 'module, ConstantPointerAuthentication> {
    /// The pointer being signed.
    #[must_use]
    pub fn pointer(self) -> Value<'context, 'module, Pointer> {
        // SAFETY: the marker guarantees what LLVM asks for.
        unsafe { Value::from_raw(raw::LLVMGetConstantPtrAuthPointer(self.raw)) }
    }

    /// The signing key.
    #[must_use]
    pub fn key(self) -> Value<'context, 'module> {
        // SAFETY: the marker guarantees what LLVM asks for.
        unsafe { Value::from_raw(raw::LLVMGetConstantPtrAuthKey(self.raw)) }
    }

    /// The constant discriminator.
    #[must_use]
    pub fn discriminator(self) -> Value<'context, 'module> {
        // SAFETY: the marker guarantees what LLVM asks for.
        unsafe { Value::from_raw(raw::LLVMGetConstantPtrAuthDiscriminator(self.raw)) }
    }

    /// The address discriminator.
    #[must_use]
    pub fn address_discriminator(self) -> Value<'context, 'module, Pointer> {
        // SAFETY: the marker guarantees what LLVM asks for.
        unsafe { Value::from_raw(raw::LLVMGetConstantPtrAuthAddrDiscriminator(self.raw)) }
    }
}

impl<'context, 'module> Value<'context, 'module, BlockAddress> {
    /// The function holding the block.
    #[must_use]
    pub fn function(self) -> Value<'context, 'module, Function> {
        // SAFETY: the marker guarantees a block address.
        unsafe { Value::from_raw(raw::LLVMGetBlockAddressFunction(self.raw)) }
    }

    /// The block.
    #[must_use]
    pub fn block(self) -> Block<'context, 'module> {
        // SAFETY: the marker guarantees a block address.
        unsafe { Block::from_raw(raw::LLVMGetBlockAddressBasicBlock(self.raw)) }
    }
}

impl<'context, 'module> Block<'context, 'module> {
    /// This block's address, as `indirectbr` takes it.
    ///
    /// # Panics
    /// If this is the function's entry block, whose address can't be taken (D3).
    #[must_use]
    #[track_caller]
    pub fn address(self) -> Value<'context, 'module, BlockAddress> {
        // SAFETY: the block is live for `'module`, and in a function.
        unsafe {
            let function = raw::LLVMGetBasicBlockParent(self.raw);
            if raw::LLVMGetEntryBasicBlock(function) == self.raw {
                bad_call(format_args!("address: the entry block's address can't be taken"));
            }
            Value::from_raw(raw::LLVMBlockAddress(function, self.raw))
        }
    }
}

/// Constant expressions over integer constants.
impl<'context, 'module, W> Value<'context, 'module, Integer<W>> {
    #[track_caller]
    fn check_constant_pair(self, operation: &str, other: Self) {
        if !self.is_constant() || !other.is_constant() {
            bad_call(format_args!(
                "{operation}: {} and {} must both be constants",
                self.print_to_string(),
                other.print_to_string()
            ));
        }
        // SAFETY: both are live.
        if !checks::same_identity(unsafe { raw::LLVMTypeOf(self.raw) } as usize, unsafe { raw::LLVMTypeOf(other.raw) }
            as usize)
        {
            bad_call(format_args!(
                "{operation}: {} and {} have different types",
                self.print_to_string(),
                other.print_to_string()
            ));
        }
    }

    #[track_caller]
    fn check_constant(self, operation: &str) {
        if !self.is_constant() {
            bad_call(format_args!("{operation}: {} is not a constant", self.print_to_string()));
        }
    }

    /// `0 - self`, as a constant.
    ///
    /// # Panics
    /// If it isn't a constant (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_negate(self) -> Self {
        self.check_constant("constant_negate");
        // SAFETY: just checked to be a constant, of an integer type.
        unsafe { Value::from_raw(raw::LLVMConstNeg(self.raw)) }
    }

    /// `0 - self`, promising no signed wrap, as a constant.
    ///
    /// # Panics
    /// If it isn't a constant (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_negate_no_signed_wrap(self) -> Self {
        self.check_constant("constant_negate_no_signed_wrap");
        // SAFETY: just checked to be a constant, of an integer type.
        unsafe { Value::from_raw(raw::LLVMConstNSWNeg(self.raw)) }
    }

    /// `0 - self`, promising no unsigned wrap, as a constant.
    ///
    /// # Panics
    /// If it isn't a constant (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_negate_no_unsigned_wrap(self) -> Self {
        self.check_constant("constant_negate_no_unsigned_wrap");
        // SAFETY: just checked to be a constant, of an integer type.
        unsafe { Value::from_raw(raw::LLVMConstNUWNeg(self.raw)) }
    }

    /// Every bit flipped, as a constant.
    ///
    /// # Panics
    /// If it isn't a constant (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_not(self) -> Self {
        self.check_constant("constant_not");
        // SAFETY: just checked to be a constant, of an integer type.
        unsafe { Value::from_raw(raw::LLVMConstNot(self.raw)) }
    }

    /// `self + other`, as a constant.
    ///
    /// # Panics
    /// If they aren't constants of one type (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_add(self, other: Self) -> Self {
        self.check_constant_pair("constant_add", other);
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstAdd(self.raw, other.raw)) }
    }

    /// `self + other`, promising no signed wrap, as a constant.
    ///
    /// # Panics
    /// If they aren't constants of one type (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_add_no_signed_wrap(self, other: Self) -> Self {
        self.check_constant_pair("constant_add_no_signed_wrap", other);
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstNSWAdd(self.raw, other.raw)) }
    }

    /// `self + other`, promising no unsigned wrap, as a constant.
    ///
    /// # Panics
    /// If they aren't constants of one type (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_add_no_unsigned_wrap(self, other: Self) -> Self {
        self.check_constant_pair("constant_add_no_unsigned_wrap", other);
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstNUWAdd(self.raw, other.raw)) }
    }

    /// `self - other`, as a constant.
    ///
    /// # Panics
    /// If they aren't constants of one type (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_subtract(self, other: Self) -> Self {
        self.check_constant_pair("constant_subtract", other);
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstSub(self.raw, other.raw)) }
    }

    /// `self - other`, promising no signed wrap, as a constant.
    ///
    /// # Panics
    /// If they aren't constants of one type (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_subtract_no_signed_wrap(self, other: Self) -> Self {
        self.check_constant_pair("constant_subtract_no_signed_wrap", other);
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstNSWSub(self.raw, other.raw)) }
    }

    /// `self - other`, promising no unsigned wrap, as a constant.
    ///
    /// # Panics
    /// If they aren't constants of one type (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_subtract_no_unsigned_wrap(self, other: Self) -> Self {
        self.check_constant_pair("constant_subtract_no_unsigned_wrap", other);
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstNUWSub(self.raw, other.raw)) }
    }

    /// `self ^ other`, as a constant.
    ///
    /// # Panics
    /// If they aren't constants of one type (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_exclusive_or(self, other: Self) -> Self {
        self.check_constant_pair("constant_exclusive_or", other);
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstXor(self.raw, other.raw)) }
    }

    /// This constant integer cast to a pointer, as a constant.
    ///
    /// # Panics
    /// If it isn't a constant (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_integer_to_pointer(self, target: Type<'context, Pointer>) -> Value<'context, 'module, Pointer> {
        self.check_constant("constant_integer_to_pointer");
        // SAFETY: an integer constant and a pointer type.
        unsafe { Value::from_raw(raw::LLVMConstIntToPtr(self.raw, target.raw)) }
    }

    /// This constant integer cut to a narrower integer, as a constant.
    ///
    /// # Panics
    /// If it isn't a constant, or the target isn't narrower (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_truncate<V>(self, target: Type<'context, Integer<V>>) -> Value<'context, 'module, Integer<V>> {
        self.check_constant("constant_truncate");
        if !checks::is_narrowing(self.width_in_bits(), target.width_in_bits()) {
            bad_call(format_args!(
                "constant_truncate: {} is not narrower than {} bits",
                target.print_to_string(),
                self.width_in_bits()
            ));
        }
        // SAFETY: an integer constant and a narrower integer type.
        unsafe { Value::from_raw(raw::LLVMConstTrunc(self.raw, target.raw)) }
    }
}

impl<'context, 'module> Value<'context, 'module, Pointer> {
    #[track_caller]
    fn check_constant_pointer(self, operation: &str) {
        if !self.is_constant() {
            bad_call(format_args!("{operation}: {} is not a constant", self.print_to_string()));
        }
    }

    /// This constant pointer cast to an integer, as a constant.
    ///
    /// # Panics
    /// If it isn't a constant (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_pointer_to_integer<W>(
        self,
        target: Type<'context, Integer<W>>,
    ) -> Value<'context, 'module, Integer<W>> {
        self.check_constant_pointer("constant_pointer_to_integer");
        // SAFETY: a pointer constant and an integer type.
        unsafe { Value::from_raw(raw::LLVMConstPtrToInt(self.raw, target.raw)) }
    }

    /// This constant pointer cast to another address space, as a constant.
    ///
    /// # Panics
    /// If it isn't a constant (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_address_space_cast(self, target: Type<'context, Pointer>) -> Self {
        self.check_constant_pointer("constant_address_space_cast");
        // SAFETY: a pointer constant and a pointer type.
        unsafe { Value::from_raw(raw::LLVMConstAddrSpaceCast(self.raw, target.raw)) }
    }

    /// A `getelementptr` over this constant pointer, as a constant. The flags say whether it is `inbounds` and so on.
    ///
    /// # Panics
    /// If this isn't a constant, an index isn't a constant integer, or the indices don't fit `element_type` (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_get_element_pointer<I>(
        self,
        element_type: Type<'context>,
        indices: &[Value<'context, 'module, I>],
        flags: crate::instructions::GetElementPointerFlags,
    ) -> Self {
        self.check_constant_pointer("constant_get_element_pointer");
        check_constants("constant_get_element_pointer", indices, None);
        let count = length_as_u32(indices.len(), "constant_get_element_pointer");
        // SAFETY: only reads them; `Value` has the layout of `LLVMValueRef`.
        let valid = unsafe {
            shim::lyftar_get_element_pointer_is_valid(element_type.raw, indices.as_ptr().cast(), count, false)
        };
        if !valid {
            bad_call(format_args!(
                "constant_get_element_pointer: those indices don't fit {}",
                element_type.print_to_string()
            ));
        }
        // SAFETY: just checked; LLVM only reads the indices.
        unsafe {
            Value::from_raw(raw::LLVMConstGEPWithNoWrapFlags(
                element_type.raw,
                self.raw,
                indices.as_ptr().cast_mut().cast(),
                count,
                flags.0,
            ))
        }
    }

    /// This constant pointer, signed for pointer authentication.
    ///
    /// # Panics
    /// If any operand isn't a constant, the key isn't an `i32`, or the discriminator isn't an `i64` (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_pointer_authentication(
        self,
        key: Value<'context, 'module, Integer<crate::types::Bits<32>>>,
        discriminator: Value<'context, 'module, Integer<crate::types::Bits<64>>>,
        address_discriminator: Value<'context, 'module, Pointer>,
    ) -> Value<'context, 'module, ConstantPointerAuthentication> {
        self.check_constant_pointer("constant_pointer_authentication");
        address_discriminator.check_constant_pointer("constant_pointer_authentication");
        if !key.is_constant() || !discriminator.is_constant() {
            bad_call(format_args!("constant_pointer_authentication: the key and discriminator must be constants"));
        }
        // SAFETY: all four were just checked to be constants of the right types.
        unsafe {
            Value::from_raw(raw::LLVMConstantPtrAuth(self.raw, key.raw, discriminator.raw, address_discriminator.raw))
        }
    }
}

impl<'context, 'module> Value<'context, 'module> {
    /// This constant cast to another type by LLVM's `bitcast`, as a constant.
    ///
    /// # Panics
    /// If this isn't a constant, or LLVM's rules don't allow that cast (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_bit_cast(self, target: Type<'context>) -> Value<'context, 'module> {
        self.check_constant_dynamic("constant_bit_cast");
        // SAFETY: only reads them.
        if !unsafe { shim::lyftar_cast_is_valid(raw::LLVMOpcode::LLVMBitCast, self.raw, target.raw) } {
            bad_call(format_args!(
                "constant_bit_cast: can't cast {} to {}",
                self.print_to_string(),
                target.print_to_string()
            ));
        }
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstBitCast(self.raw, target.raw)) }
    }

    /// This constant cut or bit-cast to another type, as a constant.
    ///
    /// # Panics
    /// If this isn't a constant, or LLVM's rules allow neither cast (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_truncate_or_bit_cast(self, target: Type<'context>) -> Value<'context, 'module> {
        self.check_constant_dynamic("constant_truncate_or_bit_cast");
        // SAFETY: only reads them.
        let valid = unsafe {
            shim::lyftar_cast_is_valid(raw::LLVMOpcode::LLVMBitCast, self.raw, target.raw)
                || shim::lyftar_cast_is_valid(raw::LLVMOpcode::LLVMTrunc, self.raw, target.raw)
        };
        if !valid {
            bad_call(format_args!(
                "constant_truncate_or_bit_cast: can't cast {} to {}",
                self.print_to_string(),
                target.print_to_string()
            ));
        }
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstTruncOrBitCast(self.raw, target.raw)) }
    }

    /// This constant cast by whichever pointer cast fits, as a constant.
    ///
    /// # Panics
    /// If this isn't a constant pointer, or the target is neither a pointer nor an integer (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_pointer_cast(self, target: Type<'context>) -> Value<'context, 'module> {
        self.check_constant_dynamic("constant_pointer_cast");
        use raw::LLVMTypeKind as K;
        if self.get_type().raw_kind() != K::LLVMPointerTypeKind
            || !matches!(target.raw_kind(), K::LLVMPointerTypeKind | K::LLVMIntegerTypeKind)
        {
            bad_call(format_args!(
                "constant_pointer_cast: can't cast {} to {}",
                self.print_to_string(),
                target.print_to_string()
            ));
        }
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstPointerCast(self.raw, target.raw)) }
    }

    #[track_caller]
    fn check_constant_dynamic(self, operation: &str) {
        if !self.is_constant() {
            bad_call(format_args!("{operation}: {} is not a constant", self.print_to_string()));
        }
    }

    /// The element of this constant vector at a constant index, as a constant.
    ///
    /// # Panics
    /// If this isn't a constant vector, or the index isn't a constant integer (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_extract_element<I>(self, index: Value<'context, 'module, I>) -> Value<'context, 'module> {
        self.check_constant_dynamic("constant_extract_element");
        // SAFETY: both live; only reads them.
        if !unsafe { shim::lyftar_extract_element_is_valid(self.raw, index.raw) } || !index.is_constant() {
            bad_call(format_args!(
                "constant_extract_element: can't take element {} of {}",
                index.print_to_string(),
                self.print_to_string()
            ));
        }
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstExtractElement(self.raw, index.raw)) }
    }

    /// This constant vector with one element replaced, as a constant.
    ///
    /// # Panics
    /// If they aren't constants, the element's type isn't the vector's, or the index isn't an integer (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_insert_element<E, I>(
        self,
        element: Value<'context, 'module, E>,
        index: Value<'context, 'module, I>,
    ) -> Value<'context, 'module> {
        self.check_constant_dynamic("constant_insert_element");
        // SAFETY: all live; only reads them.
        if !unsafe { shim::lyftar_insert_element_is_valid(self.raw, element.raw, index.raw) }
            || !element.is_constant()
            || !index.is_constant()
        {
            bad_call(format_args!(
                "constant_insert_element: can't put {} in {} at {}",
                element.print_to_string(),
                self.print_to_string(),
                index.print_to_string()
            ));
        }
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstInsertElement(self.raw, element.raw, index.raw)) }
    }

    /// Two constant vectors shuffled by a constant mask, as a constant.
    ///
    /// # Panics
    /// If they aren't constants, or LLVM's rules don't allow the shuffle (D3).
    #[must_use]
    #[track_caller]
    pub fn constant_shuffle_vector(
        self,
        other: Value<'context, 'module>,
        mask: Value<'context, 'module>,
    ) -> Value<'context, 'module> {
        self.check_constant_dynamic("constant_shuffle_vector");
        // SAFETY: all live; only reads them.
        if !unsafe { shim::lyftar_shuffle_vector_is_valid(self.raw, other.raw, mask.raw) } || !other.is_constant() {
            bad_call(format_args!(
                "constant_shuffle_vector: can't shuffle {} and {} by {}",
                self.print_to_string(),
                other.print_to_string(),
                mask.print_to_string()
            ));
        }
        // SAFETY: just checked.
        unsafe { Value::from_raw(raw::LLVMConstShuffleVector(self.raw, other.raw, mask.raw)) }
    }
}
