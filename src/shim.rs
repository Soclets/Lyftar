//! The C++ shim, for what LLVM's C API can't reach (D16). Each function here is wrapped once, by a safe function
//! elsewhere in the crate that checks its contract too (rule 9). The contracts are written in `shim/shim.cpp`.

use std::ffi::{c_char, c_int};

use crate::raw::{
    LLVMAtomicRMWBinOp, LLVMBasicBlockRef, LLVMBuilderRef, LLVMContextRef, LLVMModuleRef, LLVMOpcode, LLVMTypeRef,
    LLVMValueRef,
};

/// An LLVM `WeakVH` on the heap, owned by a `TrackedHandle`.
#[repr(C)]
pub(crate) struct WeakHandle {
    _private: [u8; 0],
}

unsafe extern "C" {
    pub(crate) fn lyftar_initialize_native_target() -> c_int;
    pub(crate) fn lyftar_initialize_all_targets();
    pub(crate) fn lyftar_initialize_named_target(name: *const c_char, length: usize) -> bool;
    pub(crate) fn lyftar_link_modules(
        destination: LLVMModuleRef,
        source: LLVMModuleRef,
        message: *mut *mut c_char,
    ) -> bool;
    pub(crate) fn lyftar_value_belongs_to_module(value: LLVMValueRef, module: LLVMModuleRef) -> bool;
    pub(crate) fn lyftar_values_share_module(user: LLVMValueRef, other: LLVMValueRef) -> bool;
    pub(crate) fn lyftar_value_is_terminator(value: LLVMValueRef) -> bool;
    pub(crate) fn lyftar_value_can_replace_uses(value: LLVMValueRef, replacement: LLVMValueRef) -> bool;
    pub(crate) fn lyftar_value_is_erasable(value: LLVMValueRef) -> bool;
    pub(crate) fn lyftar_value_is_context_constant(value: LLVMValueRef) -> bool;
    pub(crate) fn lyftar_erase_find_blocker(values: *const LLVMValueRef, count: usize) -> LLVMValueRef;
    pub(crate) fn lyftar_erase_values(values: *const LLVMValueRef, count: usize);
    pub(crate) fn lyftar_weak_handle_create(value: LLVMValueRef) -> *mut WeakHandle;
    pub(crate) fn lyftar_weak_handle_get(handle: *const WeakHandle) -> LLVMValueRef;
    pub(crate) fn lyftar_weak_handle_dispose(handle: *mut WeakHandle);

    pub(crate) fn lyftar_builder_is_open(builder: LLVMBuilderRef) -> bool;
    pub(crate) fn lyftar_builder_is_at_open_end(builder: LLVMBuilderRef) -> bool;
    pub(crate) fn lyftar_builder_can_insert_phi(builder: LLVMBuilderRef) -> bool;
    pub(crate) fn lyftar_builder_position_at_first_insertion_point(builder: LLVMBuilderRef, block: LLVMBasicBlockRef);
    pub(crate) fn lyftar_builder_move_here(builder: LLVMBuilderRef, instruction: LLVMValueRef);

    pub(crate) fn lyftar_is_valid_array_element(element: LLVMTypeRef) -> bool;
    pub(crate) fn lyftar_is_valid_vector_element(element: LLVMTypeRef) -> bool;
    pub(crate) fn lyftar_is_valid_struct_element(element: LLVMTypeRef) -> bool;
    pub(crate) fn lyftar_first_invalid_function_type_part(
        return_type: LLVMTypeRef,
        parameters: *const LLVMTypeRef,
        count: u32,
    ) -> u32;
    pub(crate) fn lyftar_type_has_null_value(checked: LLVMTypeRef) -> bool;
    pub(crate) fn lyftar_type_is_valid_global(checked: LLVMTypeRef) -> bool;
    pub(crate) fn lyftar_type_is_loadable(checked: LLVMTypeRef) -> bool;
    pub(crate) fn lyftar_type_is_first_class(checked: LLVMTypeRef) -> bool;
    pub(crate) fn lyftar_float_size_in_words(float_type: LLVMTypeRef) -> usize;
    pub(crate) fn lyftar_data_element_size_in_bits(element: LLVMTypeRef) -> u64;
    pub(crate) fn lyftar_aggregate_element_count(aggregate: LLVMTypeRef) -> u64;
    pub(crate) fn lyftar_aggregate_element_is(aggregate: LLVMTypeRef, index: u32, element: LLVMTypeRef) -> bool;
    pub(crate) fn lyftar_aggregate_matches(aggregate: LLVMTypeRef, values: *const LLVMValueRef, count: u32) -> bool;
    pub(crate) fn lyftar_inline_assembly_constraints_are_valid(
        function_type: LLVMTypeRef,
        constraints: *const c_char,
        length: usize,
    ) -> bool;

    pub(crate) fn lyftar_constant_integer_fits_unsigned(constant: LLVMValueRef) -> bool;
    pub(crate) fn lyftar_constant_integer_fits_signed(constant: LLVMValueRef) -> bool;
    pub(crate) fn lyftar_constant_data_element_count(constant: LLVMValueRef) -> u32;
    pub(crate) fn lyftar_constant_range_is_valid(width_in_bits: u32, lower: *const u64, upper: *const u64) -> bool;

    pub(crate) fn lyftar_cast_is_valid(opcode: LLVMOpcode, value: LLVMValueRef, target: LLVMTypeRef) -> bool;
    pub(crate) fn lyftar_binary_operation_is_valid(opcode: LLVMOpcode, left: LLVMValueRef, right: LLVMValueRef)
    -> bool;
    pub(crate) fn lyftar_get_element_pointer_is_valid(
        source_type: LLVMTypeRef,
        indices: *const LLVMValueRef,
        count: u32,
        allow_vector: bool,
    ) -> bool;
    pub(crate) fn lyftar_extract_element_is_valid(vector: LLVMValueRef, index: LLVMValueRef) -> bool;
    pub(crate) fn lyftar_insert_element_is_valid(
        vector: LLVMValueRef,
        element: LLVMValueRef,
        index: LLVMValueRef,
    ) -> bool;
    pub(crate) fn lyftar_shuffle_vector_is_valid(left: LLVMValueRef, right: LLVMValueRef, mask: LLVMValueRef) -> bool;
    pub(crate) fn lyftar_select_is_valid(
        condition: LLVMValueRef,
        then_value: LLVMValueRef,
        else_value: LLVMValueRef,
    ) -> bool;
    pub(crate) fn lyftar_call_first_bad_argument(
        function_type: LLVMTypeRef,
        arguments: *const LLVMValueRef,
        count: u32,
    ) -> u32;
    pub(crate) fn lyftar_atomic_read_modify_write_is_valid(
        operation: LLVMAtomicRMWBinOp,
        value_type: LLVMTypeRef,
    ) -> bool;
    pub(crate) fn lyftar_token_none(context: LLVMContextRef) -> LLVMValueRef;

    pub(crate) fn lyftar_call_set_called_function(call: LLVMValueRef, function: LLVMValueRef);
    pub(crate) fn lyftar_call_does_not_capture(call: LLVMValueRef, index: u32) -> bool;
    pub(crate) fn lyftar_intrinsic_is_lifetime_start_or_end(call: LLVMValueRef) -> bool;
    pub(crate) fn lyftar_intrinsic_overloads_match(id: u32, overloads: *const LLVMTypeRef, count: usize) -> bool;
    pub(crate) fn lyftar_allocation_size_in_bytes(allocation: LLVMValueRef, size_in_bytes: *mut u64) -> bool;
    pub(crate) fn lyftar_function_instruction_count(function: LLVMValueRef) -> u32;

    pub(crate) fn lyftar_data_layout_is_valid(text: *const c_char) -> bool;
    pub(crate) fn lyftar_attribute_kind_takes_integer_or_nothing(kind: u32, value: u64) -> bool;
    pub(crate) fn lyftar_attribute_kind_takes_type(kind: u32) -> bool;
    pub(crate) fn lyftar_attribute_kind_takes_constant_range(kind: u32) -> bool;
}
