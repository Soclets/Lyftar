//! The C++ shim, for what LLVM's C API can't reach (D16). Each function here is wrapped once, by a safe function
//! elsewhere in the crate that checks its contract too (rule 9). The contracts are written in `shim/shim.cpp`.

use std::ffi::{c_char, c_int, c_void};

use crate::raw::{
    LLVMAtomicRMWBinOp, LLVMBasicBlockRef, LLVMBuilderRef, LLVMContextRef, LLVMModuleRef, LLVMOpcode, LLVMTypeRef,
    LLVMValueRef,
};

/// An LLVM `WeakVH` on the heap, owned by a `TrackedHandle`.
#[repr(C)]
pub struct WeakHandle {
    _private: [u8; 0],
}

// trace: D52
#[cfg(lyftar_has_lld)]
unsafe extern "C" {
    pub fn lyftar_lld(
        arguments: *const *const c_char,
        count: usize,
        output: *mut *mut c_char,
        can_run_again: *mut bool,
    ) -> c_int;
    pub fn lyftar_object_symbols(
        bytes: *const c_char,
        size: usize,
        defined: *mut *mut c_char,
        undefined: *mut *mut c_char,
    ) -> bool;
}

unsafe extern "C" {
    pub fn lyftar_initialize_native_target() -> c_int;
    pub fn lyftar_initialize_all_targets();
    pub fn lyftar_target_was_built(name: *const c_char, length: usize) -> bool;
    pub fn lyftar_parse_ir(
        context: LLVMContextRef,
        buffer: crate::raw::LLVMMemoryBufferRef,
        module: *mut LLVMModuleRef,
        message: *mut *mut c_char,
    ) -> crate::raw::LLVMBool;
    pub fn lyftar_summary_file_was_given() -> bool;
    pub fn lyftar_verify_function(function: LLVMValueRef) -> *mut c_char;
    pub fn lyftar_strategy_is_registered(name: *const c_char, length: usize) -> bool;
    pub fn lyftar_strategy_manages_pointer(function: LLVMValueRef, pointer_type: LLVMTypeRef) -> c_int;
    pub fn lyftar_module_strategy_problem(module: LLVMModuleRef) -> *mut c_char;
    pub fn lyftar_strategy_register(
        name: *const c_char,
        name_length: usize,
        description: *const c_char,
        description_length: usize,
        rewrite: bool,
        manages: unsafe extern "C" fn(*const c_void, LLVMTypeRef) -> c_int,
        state: *const c_void,
    );
    pub fn lyftar_module_needs_safepoint_poll(module: LLVMModuleRef) -> bool;
    pub fn lyftar_module_has_transition_statepoint(module: LLVMModuleRef) -> bool;
    pub fn lyftar_jit_capture_stack_maps(jit: crate::raw::LLVMOrcLLJITRef) -> *mut c_void;
    pub fn lyftar_execution_engine_capture_stack_maps(engine: crate::raw::LLVMExecutionEngineRef) -> *mut c_void;
    pub fn lyftar_execution_engine_finish_code(engine: crate::raw::LLVMExecutionEngineRef);
    pub fn lyftar_stack_maps_visit(
        captured: *mut c_void,
        visitor: unsafe extern "C" fn(*mut c_void, *const u8, usize),
        state: *mut c_void,
    );
    pub fn lyftar_stack_maps_dispose(captured: *mut c_void);
    pub fn lyftar_module_needs_elf_for_its_tables(module: LLVMModuleRef, triple: *const c_char) -> bool;
    pub fn lyftar_value_is_static_stack_allocation(value: LLVMValueRef) -> bool;
    pub fn lyftar_value_is_stack_allocation(value: LLVMValueRef) -> bool;
    pub fn lyftar_stack_allocation_holds_a_pointer(value: LLVMValueRef) -> bool;
    pub fn lyftar_value_is_statepoint(value: LLVMValueRef) -> bool;
    pub fn lyftar_block_unique_predecessor(block: LLVMBasicBlockRef) -> LLVMBasicBlockRef;
    pub fn lyftar_module_has_pointer_authentication(module: LLVMModuleRef) -> bool;
    pub fn lyftar_link_modules(destination: LLVMModuleRef, source: LLVMModuleRef, message: *mut *mut c_char) -> bool;
    pub fn lyftar_value_belongs_to_module(value: LLVMValueRef, module: LLVMModuleRef) -> bool;
    pub fn lyftar_values_share_module(user: LLVMValueRef, other: LLVMValueRef) -> bool;
    pub fn lyftar_value_operands_can_change(value: LLVMValueRef) -> bool;
    pub fn lyftar_value_can_replace_uses(value: LLVMValueRef, replacement: LLVMValueRef) -> bool;
    pub fn lyftar_value_has_constant_user(value: LLVMValueRef) -> bool;
    pub fn lyftar_replacing_keeps_metadata(value: LLVMValueRef, replacement: LLVMValueRef) -> bool;
    pub fn lyftar_replace_find_conflict(
        old_values: *const LLVMValueRef,
        new_values: *const LLVMValueRef,
        count: usize,
    ) -> LLVMValueRef;
    pub fn lyftar_value_is_erasable(value: LLVMValueRef) -> bool;
    pub fn lyftar_value_is_context_constant(value: LLVMValueRef) -> bool;
    pub fn lyftar_value_is_in_function(value: LLVMValueRef, function: LLVMValueRef) -> bool;
    pub fn lyftar_erase_find_blocker(values: *const LLVMValueRef, count: usize) -> LLVMValueRef;
    pub fn lyftar_erase_values(values: *const LLVMValueRef, count: usize);
    pub fn lyftar_weak_handle_create(value: LLVMValueRef) -> *mut WeakHandle;
    pub fn lyftar_weak_handle_get(handle: *const WeakHandle) -> LLVMValueRef;
    pub fn lyftar_weak_handle_dispose(handle: *mut WeakHandle);

    pub fn lyftar_builder_is_open(builder: LLVMBuilderRef) -> bool;
    pub fn lyftar_builder_is_at_open_end(builder: LLVMBuilderRef) -> bool;
    pub fn lyftar_builder_can_insert_phi(builder: LLVMBuilderRef) -> bool;
    pub fn lyftar_builder_position_at_first_insertion_point(builder: LLVMBuilderRef, block: LLVMBasicBlockRef);
    pub fn lyftar_builder_move_here(builder: LLVMBuilderRef, instruction: LLVMValueRef);

    pub fn lyftar_is_valid_array_element(element: LLVMTypeRef) -> bool;
    pub fn lyftar_is_valid_vector_element(element: LLVMTypeRef) -> bool;
    pub fn lyftar_is_valid_struct_element(element: LLVMTypeRef) -> bool;
    pub fn lyftar_first_invalid_function_type_part(
        return_type: LLVMTypeRef,
        parameters: *const LLVMTypeRef,
        count: u32,
    ) -> u32;
    pub fn lyftar_type_has_null_value(checked: LLVMTypeRef) -> bool;
    pub fn lyftar_target_extension_type_is_valid(
        context: LLVMContextRef,
        name: *const c_char,
        type_parameters: *const LLVMTypeRef,
        type_parameter_count: u32,
        integer_parameters: *const u32,
        integer_parameter_count: u32,
    ) -> bool;
    pub fn lyftar_type_is_valid_global(checked: LLVMTypeRef) -> bool;
    pub fn lyftar_type_is_loadable(checked: LLVMTypeRef) -> bool;
    pub fn lyftar_type_is_first_class(checked: LLVMTypeRef) -> bool;
    pub fn lyftar_float_size_in_words(float_type: LLVMTypeRef) -> usize;
    pub fn lyftar_data_element_size_in_bits(element: LLVMTypeRef) -> u64;
    pub fn lyftar_aggregate_element_count(aggregate: LLVMTypeRef) -> u64;
    pub fn lyftar_aggregate_element_is(aggregate: LLVMTypeRef, index: u32, element: LLVMTypeRef) -> bool;
    pub fn lyftar_aggregate_matches(aggregate: LLVMTypeRef, values: *const LLVMValueRef, count: u32) -> bool;
    pub fn lyftar_inline_assembly_constraints_are_valid(
        function_type: LLVMTypeRef,
        constraints: *const c_char,
        length: usize,
    ) -> bool;

    pub fn lyftar_constant_integer_fits_unsigned(constant: LLVMValueRef) -> bool;
    pub fn lyftar_constant_integer_fits_signed(constant: LLVMValueRef) -> bool;
    pub fn lyftar_constant_data_element_count(constant: LLVMValueRef) -> u32;
    pub fn lyftar_constant_range_is_valid(width_in_bits: u32, lower: *const u64, upper: *const u64) -> bool;

    pub fn lyftar_cast_is_valid(opcode: LLVMOpcode, value: LLVMValueRef, target: LLVMTypeRef) -> bool;
    pub fn lyftar_binary_operation_is_valid(opcode: LLVMOpcode, left: LLVMValueRef, right: LLVMValueRef) -> bool;
    pub fn lyftar_get_element_pointer_is_valid(
        source_type: LLVMTypeRef,
        indices: *const LLVMValueRef,
        count: u32,
        allow_vector: bool,
    ) -> bool;
    pub fn lyftar_extract_element_is_valid(vector: LLVMValueRef, index: LLVMValueRef) -> bool;
    pub fn lyftar_insert_element_is_valid(vector: LLVMValueRef, element: LLVMValueRef, index: LLVMValueRef) -> bool;
    pub fn lyftar_shuffle_vector_is_valid(left: LLVMValueRef, right: LLVMValueRef, mask: LLVMValueRef) -> bool;
    pub fn lyftar_select_is_valid(condition: LLVMValueRef, then_value: LLVMValueRef, else_value: LLVMValueRef) -> bool;
    pub fn lyftar_call_first_bad_argument(
        function_type: LLVMTypeRef,
        arguments: *const LLVMValueRef,
        count: u32,
    ) -> u32;
    pub fn lyftar_atomic_read_modify_write_is_valid(operation: LLVMAtomicRMWBinOp, value_type: LLVMTypeRef) -> bool;
    pub fn lyftar_token_none(context: LLVMContextRef) -> LLVMValueRef;

    pub fn lyftar_call_set_called_function(call: LLVMValueRef, function: LLVMValueRef);
    pub fn lyftar_call_does_not_capture(call: LLVMValueRef, index: u32) -> bool;
    pub fn lyftar_intrinsic_is_lifetime_start_or_end(call: LLVMValueRef) -> bool;
    pub fn lyftar_intrinsic_overloads_match(id: u32, overloads: *const LLVMTypeRef, count: usize) -> bool;
    pub fn lyftar_allocation_size_in_bytes(allocation: LLVMValueRef, size_in_bytes: *mut u64) -> bool;
    pub fn lyftar_function_instruction_count(function: LLVMValueRef) -> u32;

    pub fn lyftar_data_layout_is_valid(text: *const c_char) -> bool;
    pub fn lyftar_attribute_kind_takes_integer_or_nothing(kind: u32, value: u64) -> bool;
    pub fn lyftar_attribute_kind_takes_type(kind: u32) -> bool;
    pub fn lyftar_attribute_kind_takes_constant_range(kind: u32) -> bool;
    pub fn lyftar_debug_record_is_trailing(record: crate::raw::LLVMDbgRecordRef) -> bool;
    pub fn lyftar_metadata_node_can_change_operand(metadata: crate::raw::LLVMMetadataRef) -> bool;
    pub fn lyftar_module_materialize_all(module: LLVMModuleRef) -> crate::raw::LLVMErrorRef;
    pub fn lyftar_relocation_type_name(relocation: crate::raw::LLVMRelocationIteratorRef) -> *mut c_char;
    pub fn lyftar_metadata_is(metadata: crate::raw::LLVMMetadataRef, which: u32) -> bool;
    pub fn lyftar_temporary_metadata_dispose(metadata: crate::raw::LLVMMetadataRef);
}

/// What a pass gets told about how a pass manager is set up; the Rust side of `LyftarPassOptions` in `shim/passes.cpp`.
#[repr(C)]
pub struct PassOptionsForShim {
    pub verify_each: bool,
    pub debug_logging: bool,
    pub loop_interleaving: bool,
    pub loop_vectorization: bool,
    pub superword_level_parallelism_vectorization: bool,
    pub loop_unrolling: bool,
    pub forget_all_scalar_evolution_in_loop_unroll: bool,
    pub call_graph_profile: bool,
    pub merge_functions: bool,
    pub licm_memory_ssa_optimization_cap: u32,
    pub licm_memory_ssa_no_access_for_promotion_cap: u32,
    pub inliner_threshold: i32,
    pub alias_analysis_pipeline: *const c_char,
    pub alias_analysis_pipeline_length: usize,
}

/// The struct a pass plugin hands to `opt` or `clang`: LLVM's `PassPluginLibraryInfo`.
#[repr(C)]
pub struct PluginInformation {
    pub api_version: u32,
    pub plugin_name: *const c_char,
    pub plugin_version: *const c_char,
    pub register_callbacks: extern "C" fn(*mut std::ffi::c_void),
    pub pre_code_generation_callback: *const std::ffi::c_void,
}

pub type RunModulePass = unsafe extern "C" fn(*mut std::ffi::c_void, LLVMModuleRef, *mut std::ffi::c_void) -> bool;
pub type RunFunctionPass = unsafe extern "C" fn(*mut std::ffi::c_void, LLVMValueRef, *mut std::ffi::c_void) -> bool;
pub type Dispose = unsafe extern "C" fn(*mut std::ffi::c_void);
pub type MakePass = unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void);
pub type Register = unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void);
pub type ExtensionPointCallback =
    unsafe extern "C" fn(*mut std::ffi::c_void, *mut std::ffi::c_void, *mut std::ffi::c_void, u32, u32);

unsafe extern "C" {
    pub fn lyftar_run_passes_with(
        module: LLVMModuleRef,
        pipeline: *const c_char,
        length: usize,
        machine: crate::raw::LLVMTargetMachineRef,
        options: *const PassOptionsForShim,
        state: *mut std::ffi::c_void,
        register: Register,
    ) -> crate::raw::LLVMErrorRef;

    pub fn lyftar_pass_builder_register_module_pass(
        builder: *mut std::ffi::c_void,
        name: *const c_char,
        length: usize,
        state: *mut std::ffi::c_void,
        make: MakePass,
        dispose: Dispose,
    );
    pub fn lyftar_pass_builder_register_function_pass(
        builder: *mut std::ffi::c_void,
        name: *const c_char,
        length: usize,
        state: *mut std::ffi::c_void,
        make: MakePass,
        dispose: Dispose,
    );
    pub fn lyftar_pass_builder_register_extension_point(
        builder: *mut std::ffi::c_void,
        which: u32,
        state: *mut std::ffi::c_void,
        call: ExtensionPointCallback,
        dispose: Dispose,
    );

    pub fn lyftar_module_pass_manager_add_module_pass(
        manager: *mut std::ffi::c_void,
        state: *mut std::ffi::c_void,
        run: RunModulePass,
        dispose: Dispose,
    );
    pub fn lyftar_module_pass_manager_add_function_pass(
        manager: *mut std::ffi::c_void,
        state: *mut std::ffi::c_void,
        run: RunFunctionPass,
        dispose: Dispose,
    );
    pub fn lyftar_function_pass_manager_add_function_pass(
        manager: *mut std::ffi::c_void,
        state: *mut std::ffi::c_void,
        run: RunFunctionPass,
        dispose: Dispose,
    );
    pub fn lyftar_call_graph_pass_manager_add_function_pass(
        manager: *mut std::ffi::c_void,
        state: *mut std::ffi::c_void,
        run: RunFunctionPass,
        dispose: Dispose,
    );
    pub fn lyftar_pass_manager_add_pipeline(
        builder: *mut std::ffi::c_void,
        manager: *mut std::ffi::c_void,
        which: u32,
        pipeline: *const c_char,
        length: usize,
    ) -> crate::raw::LLVMErrorRef;

    pub fn lyftar_analyses_clear(analyses: *mut std::ffi::c_void);
    pub fn lyftar_analyses_dominator_tree(
        analyses: *mut std::ffi::c_void,
        function: LLVMValueRef,
    ) -> *mut std::ffi::c_void;
    pub fn lyftar_dominator_tree_root(tree: *mut std::ffi::c_void) -> LLVMBasicBlockRef;
    pub fn lyftar_dominator_tree_block_dominates(
        tree: *mut std::ffi::c_void,
        dominating: LLVMBasicBlockRef,
        dominated: LLVMBasicBlockRef,
    ) -> bool;
    pub fn lyftar_dominator_tree_block_properly_dominates(
        tree: *mut std::ffi::c_void,
        dominating: LLVMBasicBlockRef,
        dominated: LLVMBasicBlockRef,
    ) -> bool;
    pub fn lyftar_dominator_tree_value_dominates(
        tree: *mut std::ffi::c_void,
        definition: LLVMValueRef,
        user: LLVMValueRef,
    ) -> bool;
    pub fn lyftar_dominator_tree_immediate_dominator(
        tree: *mut std::ffi::c_void,
        block: LLVMBasicBlockRef,
    ) -> LLVMBasicBlockRef;
    pub fn lyftar_dominator_tree_is_reachable(tree: *mut std::ffi::c_void, block: LLVMBasicBlockRef) -> bool;
    pub fn lyftar_dominator_tree_child_count(tree: *mut std::ffi::c_void, block: LLVMBasicBlockRef) -> u32;
    pub fn lyftar_dominator_tree_child_at(
        tree: *mut std::ffi::c_void,
        block: LLVMBasicBlockRef,
        index: u32,
    ) -> LLVMBasicBlockRef;

    pub fn lyftar_analyses_loop_info(analyses: *mut std::ffi::c_void, function: LLVMValueRef) -> *mut std::ffi::c_void;
    pub fn lyftar_loop_info_loop_for(
        information: *mut std::ffi::c_void,
        block: LLVMBasicBlockRef,
    ) -> *mut std::ffi::c_void;
    pub fn lyftar_loop_info_top_level_count(information: *mut std::ffi::c_void) -> u32;
    pub fn lyftar_loop_info_top_level_at(information: *mut std::ffi::c_void, index: u32) -> *mut std::ffi::c_void;
    pub fn lyftar_loop_header(found: *mut std::ffi::c_void) -> LLVMBasicBlockRef;
    pub fn lyftar_loop_preheader(found: *mut std::ffi::c_void) -> LLVMBasicBlockRef;
    pub fn lyftar_loop_latch(found: *mut std::ffi::c_void) -> LLVMBasicBlockRef;
    pub fn lyftar_loop_parent(found: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    pub fn lyftar_loop_depth(found: *mut std::ffi::c_void) -> u32;
    pub fn lyftar_loop_contains_block(found: *mut std::ffi::c_void, block: LLVMBasicBlockRef) -> bool;
    pub fn lyftar_loop_block_count(found: *mut std::ffi::c_void) -> u32;
    pub fn lyftar_loop_block_at(found: *mut std::ffi::c_void, index: u32) -> LLVMBasicBlockRef;
    pub fn lyftar_loop_subloop_count(found: *mut std::ffi::c_void) -> u32;
    pub fn lyftar_loop_subloop_at(found: *mut std::ffi::c_void, index: u32) -> *mut std::ffi::c_void;
    pub fn lyftar_loop_exit_blocks(found: *mut std::ffi::c_void, buffer: *mut LLVMBasicBlockRef, capacity: u32) -> u32;

    pub fn lyftar_clone_function(function: LLVMValueRef) -> LLVMValueRef;
    pub fn lyftar_inline_call(call: LLVMValueRef, message: *mut *mut c_char) -> bool;
    pub fn lyftar_structural_hash_of_module(module: LLVMModuleRef) -> u64;
    pub fn lyftar_structural_hash_of_function(function: LLVMValueRef) -> u64;
}
