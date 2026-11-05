//! The C++ shim, for what LLVM's C API can't reach (D16). Each function here is wrapped once, by a safe function
//! elsewhere in the crate that checks its contract too (rule 9). The contracts are written in `shim/shim.cpp`.

use crate::raw::{LLVMBuilderRef, LLVMModuleRef, LLVMValueRef};

unsafe extern "C" {
    pub(crate) fn lyftar_initialize_native_target() -> std::ffi::c_int;
    pub(crate) fn lyftar_value_belongs_to_module(value: LLVMValueRef, module: LLVMModuleRef) -> bool;
    pub(crate) fn lyftar_builder_is_open(builder: LLVMBuilderRef) -> bool;
}
