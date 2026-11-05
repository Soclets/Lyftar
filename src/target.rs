//! Targets, target machines and object emission.

use std::marker::PhantomData;
use std::path::Path;
use std::sync::OnceLock;

use crate::bad_call::bad_call;
use crate::checks;
use crate::context::Context;
use crate::enums::llvm_enum;
use crate::error::Error;
use crate::memory_buffer::MemoryBuffer;
use crate::module::Module;
use crate::raw;
use crate::shim;
use crate::text::{c_path, c_string, from_borrowed, from_owned_message};
use crate::types::{Integer, Type};
use crate::values::{GlobalVariable, Value};

llvm_enum! {
    /// How hard the code generator tries.
    pub enum CodeGenerationOptimizationLevel: LLVMCodeGenOptLevel {
        /// Not at all.
        None = LLVMCodeGenLevelNone,
        /// A little.
        Less = LLVMCodeGenLevelLess,
        /// The default.
        Default = LLVMCodeGenLevelDefault,
        /// As hard as it can.
        Aggressive = LLVMCodeGenLevelAggressive,
    }
}

llvm_enum! {
    /// How the code expects to be placed in memory.
    pub enum RelocationMode: LLVMRelocMode {
        /// The target's default.
        Default = LLVMRelocDefault,
        /// Non-relocatable.
        Static = LLVMRelocStatic,
        /// Position-independent.
        PositionIndependentCode = LLVMRelocPIC,
        /// Dynamic with no position-independent code.
        DynamicNoPositionIndependentCode = LLVMRelocDynamicNoPic,
        /// Read-only data is position-independent.
        ReadOnlyPositionIndependent = LLVMRelocROPI,
        /// Read-write data is position-independent.
        ReadWritePositionIndependent = LLVMRelocRWPI,
        /// Both kinds of data are position-independent.
        ReadOnlyAndReadWritePositionIndependent = LLVMRelocROPI_RWPI,
    }
}

llvm_enum! {
    /// What the code assumes about how far apart things are.
    pub enum CodeModel: LLVMCodeModel {
        /// The target's default.
        Default = LLVMCodeModelDefault,
        /// The target's default for a JIT.
        JitDefault = LLVMCodeModelJITDefault,
        /// Tiny.
        Tiny = LLVMCodeModelTiny,
        /// Small.
        Small = LLVMCodeModelSmall,
        /// Kernel.
        Kernel = LLVMCodeModelKernel,
        /// Medium.
        Medium = LLVMCodeModelMedium,
        /// Large.
        Large = LLVMCodeModelLarge,
    }
}

llvm_enum! {
    /// What code generation writes.
    pub enum CodeGenerationFileType: LLVMCodeGenFileType {
        /// Assembly text.
        Assembly = LLVMAssemblyFile,
        /// An object file.
        Object = LLVMObjectFile,
    }
}

llvm_enum! {
    /// What `GlobalISel` does when it can't handle something.
    pub enum GlobalInstructionSelectionAbortMode: LLVMGlobalISelAbortMode {
        /// Fall back to the other selector, quietly.
        Disable = LLVMGlobalISelAbortDisable,
        /// Stop.
        Enable = LLVMGlobalISelAbortEnable,
        /// Fall back to the other selector, with a message.
        DisableWithDiagnostic = LLVMGlobalISelAbortDisableWithDiag,
    }
}

llvm_enum! {
    /// The order of a value's bytes in memory.
    pub enum ByteOrdering: LLVMByteOrdering {
        /// Most significant byte first.
        BigEndian = LLVMBigEndian,
        /// Least significant byte first.
        LittleEndian = LLVMLittleEndian,
    }
}

static TARGETS: OnceLock<bool> = OnceLock::new();

/// Sets up LLVM's target registry, once in the life of the process, and says whether it has a target for this machine.
///
/// LLVM's registry takes no lock. Two threads registering one target tie its list into a loop, and every lookup after that
/// spins for good. `libLTO` registers all of them the first time it is used, with no lock either. So everything is
/// registered here, `libLTO`'s turn included, and nothing writes to the registry again. Everything that can read the
/// registry calls this first. (`ASSUMPTIONS.md`)
pub(crate) fn set_up_targets() -> bool {
    *TARGETS.get_or_init(|| {
        // SAFETY: these take nothing, and `OnceLock` keeps them to one call in one thread. The generator exists only to have
        // `libLTO` run its own setup now, and the wrapper for each `libLTO` call waits for this.
        unsafe {
            shim::lyftar_initialize_all_targets();
            let native_failed = shim::lyftar_initialize_native_target() != 0;
            raw::lto_codegen_dispose(raw::lto_codegen_create_in_local_context());
            !native_failed
        }
    })
}

/// Sets up this machine's own target. Everything Lyftar does sets this up by itself, so this is only needed to ask whether
/// LLVM has one. It is safe to call more than once.
///
/// # Errors
/// If LLVM has no target for this machine.
pub fn initialize_native_target() -> Result<(), Error> {
    if set_up_targets() { Ok(()) } else { Err(Error::new("LLVM has no target for this machine")) }
}

/// Sets up every target LLVM was built with. Everything Lyftar does sets them up by itself, so this is never needed first.
/// It is safe to call more than once.
pub fn initialize_all_targets() {
    set_up_targets();
}

/// Sets up the target LLVM calls `name`, such as `AArch64`. Everything Lyftar does sets it up by itself, so this is only
/// needed to ask whether LLVM has it. It is safe to call more than once.
///
/// # Errors
/// If LLVM wasn't built with that target.
pub fn initialize_target(name: &str) -> Result<(), Error> {
    set_up_targets();
    // SAFETY: the shim reads `name.len()` bytes.
    if unsafe { shim::lyftar_target_was_built(name.as_ptr().cast(), name.len()) } {
        Ok(())
    } else {
        Err(Error::new(format!("LLVM wasn't built with the target {name}")))
    }
}

/// The triple of the machine this is running on.
#[must_use]
pub fn default_target_triple() -> String {
    // SAFETY: takes nothing, and LLVM allocates the text for the caller to free.
    from_owned_message(unsafe { raw::LLVMGetDefaultTargetTriple() })
}

/// A triple in its normal form.
///
/// # Panics
/// If `triple` has a NUL byte inside it (D3).
#[must_use]
#[track_caller]
pub fn normalize_target_triple(triple: &str) -> String {
    let triple = c_string(triple, "normalize_target_triple");
    // SAFETY: a valid C string; LLVM allocates the text for the caller to free.
    from_owned_message(unsafe { raw::LLVMNormalizeTargetTriple(triple.as_ptr()) })
}

/// The name of this machine's processor.
#[must_use]
pub fn host_cpu_name() -> String {
    // SAFETY: takes nothing, and LLVM allocates the text for the caller to free.
    from_owned_message(unsafe { raw::LLVMGetHostCPUName() })
}

/// The features of this machine's processor.
#[must_use]
pub fn host_cpu_features() -> String {
    // SAFETY: takes nothing, and LLVM allocates the text for the caller to free.
    from_owned_message(unsafe { raw::LLVMGetHostCPUFeatures() })
}

/// A target: a family of machines LLVM can make code for. It lives in LLVM's global registry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Target {
    raw: raw::LLVMTargetRef,
}

impl Target {
    /// The registered targets, which is none until something is initialized.
    #[must_use]
    pub fn all() -> Vec<Target> {
        let mut targets = Vec::new();
        // SAFETY: takes nothing, and each next target comes from the last.
        set_up_targets();
        // SAFETY: takes nothing, and the registry is set up.
        let mut next = unsafe { raw::LLVMGetFirstTarget() };
        while !next.is_null() {
            targets.push(Target { raw: next });
            // SAFETY: LLVM just gave this one back.
            next = unsafe { raw::LLVMGetNextTarget(next) };
        }
        targets
    }

    /// The target called `name`, if it is registered.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn from_name(name: &str) -> Option<Target> {
        let name = c_string(name, "from_name");
        // SAFETY: a valid C string for the call.
        let raw_target = unsafe { raw::LLVMGetTargetFromName(name.as_ptr()) };
        (!raw_target.is_null()).then_some(Target { raw: raw_target })
    }

    /// The target that makes code for `triple`.
    ///
    /// # Errors
    /// If no registered target does.
    ///
    /// # Panics
    /// If `triple` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn from_triple(triple: &str) -> Result<Target, Error> {
        let triple = c_string(triple, "from_triple");
        let mut found = std::ptr::null_mut();
        let mut message = std::ptr::null_mut();
        // SAFETY: a valid C string; LLVM writes the target, or a message for the caller to free.
        let failed = unsafe { raw::LLVMGetTargetFromTriple(triple.as_ptr(), &raw mut found, &raw mut message) };
        if failed != 0 {
            return Err(Error::from_owned_message(message));
        }
        Ok(Target { raw: found })
    }

    /// This target's name.
    #[must_use]
    pub fn name(self) -> String {
        // SAFETY: a target lives in LLVM's registry for good.
        from_borrowed(unsafe { raw::LLVMGetTargetName(self.raw) })
    }

    /// This target's description.
    #[must_use]
    pub fn description(self) -> String {
        // SAFETY: a target lives in LLVM's registry for good.
        from_borrowed(unsafe { raw::LLVMGetTargetDescription(self.raw) })
    }

    /// Whether this target has a JIT.
    #[must_use]
    pub fn has_jit(self) -> bool {
        // SAFETY: a target lives in LLVM's registry for good.
        unsafe { raw::LLVMTargetHasJIT(self.raw) != 0 }
    }

    /// Whether this target can make a target machine.
    #[must_use]
    pub fn has_target_machine(self) -> bool {
        // SAFETY: a target lives in LLVM's registry for good.
        unsafe { raw::LLVMTargetHasTargetMachine(self.raw) != 0 }
    }

    /// Whether this target has an assembly backend.
    #[must_use]
    pub fn has_assembly_backend(self) -> bool {
        // SAFETY: a target lives in LLVM's registry for good.
        unsafe { raw::LLVMTargetHasAsmBackend(self.raw) != 0 }
    }

    /// A target machine for `triple`.
    ///
    /// # Panics
    /// If a string has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn create_target_machine(
        self,
        triple: &str,
        cpu: &str,
        features: &str,
        level: CodeGenerationOptimizationLevel,
        relocation_mode: RelocationMode,
        code_model: CodeModel,
    ) -> TargetMachine {
        let triple = c_string(triple, "create_target_machine");
        let cpu = c_string(cpu, "create_target_machine");
        let features = c_string(features, "create_target_machine");
        // SAFETY: a target lives for good, and the strings are valid C strings for the call.
        let raw_machine = unsafe {
            raw::LLVMCreateTargetMachine(
                self.raw,
                triple.as_ptr(),
                cpu.as_ptr(),
                features.as_ptr(),
                level.to_raw(),
                relocation_mode.to_raw(),
                code_model.to_raw(),
            )
        };
        TargetMachine { raw: raw_machine }
    }

    /// A target machine for `triple`, from a set of options.
    ///
    /// # Panics
    /// If `triple` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn create_target_machine_with_options(self, triple: &str, options: &TargetMachineOptions) -> TargetMachine {
        let triple = c_string(triple, "create_target_machine_with_options");
        // SAFETY: a target lives for good, the string is valid for the call, and LLVM only reads the options.
        let raw_machine = unsafe { raw::LLVMCreateTargetMachineWithOptions(self.raw, triple.as_ptr(), options.raw) };
        TargetMachine { raw: raw_machine }
    }
}

/// What to give a new target machine, for [`Target::create_target_machine_with_options`].
pub struct TargetMachineOptions {
    raw: raw::LLVMTargetMachineOptionsRef,
}

impl Drop for TargetMachineOptions {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the options.
        unsafe { raw::LLVMDisposeTargetMachineOptions(self.raw) };
    }
}

impl Default for TargetMachineOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl TargetMachineOptions {
    /// The default options.
    #[must_use]
    pub fn new() -> Self {
        // SAFETY: takes nothing.
        TargetMachineOptions { raw: unsafe { raw::LLVMCreateTargetMachineOptions() } }
    }

    /// Sets the processor.
    ///
    /// # Panics
    /// If `cpu` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn set_cpu(&mut self, cpu: &str) {
        let cpu = c_string(cpu, "set_cpu");
        // SAFETY: the options are live, and LLVM copies the valid C string.
        unsafe { raw::LLVMTargetMachineOptionsSetCPU(self.raw, cpu.as_ptr()) };
    }

    /// Sets the features.
    ///
    /// # Panics
    /// If `features` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn set_features(&mut self, features: &str) {
        let features = c_string(features, "set_features");
        // SAFETY: the options are live, and LLVM copies the valid C string.
        unsafe { raw::LLVMTargetMachineOptionsSetFeatures(self.raw, features.as_ptr()) };
    }

    /// Sets the binary interface.
    ///
    /// # Panics
    /// If `abi` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn set_abi(&mut self, abi: &str) {
        let abi = c_string(abi, "set_abi");
        // SAFETY: the options are live, and LLVM copies the valid C string.
        unsafe { raw::LLVMTargetMachineOptionsSetABI(self.raw, abi.as_ptr()) };
    }

    /// Sets how hard the code generator tries.
    pub fn set_optimization_level(&mut self, level: CodeGenerationOptimizationLevel) {
        // SAFETY: the options are live.
        unsafe { raw::LLVMTargetMachineOptionsSetCodeGenOptLevel(self.raw, level.to_raw()) };
    }

    /// Sets how the code expects to be placed in memory.
    pub fn set_relocation_mode(&mut self, mode: RelocationMode) {
        // SAFETY: the options are live.
        unsafe { raw::LLVMTargetMachineOptionsSetRelocMode(self.raw, mode.to_raw()) };
    }

    /// Sets what the code assumes about how far apart things are.
    pub fn set_code_model(&mut self, model: CodeModel) {
        // SAFETY: the options are live.
        unsafe { raw::LLVMTargetMachineOptionsSetCodeModel(self.raw, model.to_raw()) };
    }
}

/// A machine LLVM makes code for. It doesn't belong to a context, so it can be kept across [`crate::with_context`].
pub struct TargetMachine {
    pub(crate) raw: raw::LLVMTargetMachineRef,
}

impl Drop for TargetMachine {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the machine.
        unsafe { raw::LLVMDisposeTargetMachine(self.raw) };
    }
}

impl TargetMachine {
    /// Takes ownership of a raw target machine.
    ///
    /// # Safety
    /// `raw` must be a live target machine that nothing else owns.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMTargetMachineRef) -> Self {
        TargetMachine { raw }
    }

    /// The raw target machine.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(&self) -> raw::LLVMTargetMachineRef {
        self.raw
    }

    pub(crate) fn into_raw(self) -> raw::LLVMTargetMachineRef {
        let raw = self.raw;
        std::mem::forget(self);
        raw
    }

    /// A target machine for this very computer, with position-independent code and the most optimization.
    ///
    /// # Errors
    /// If LLVM has no target for this machine.
    pub fn for_host() -> Result<TargetMachine, Error> {
        initialize_native_target()?;
        let triple = default_target_triple();
        let target = Target::from_triple(&triple)?;
        Ok(target.create_target_machine(
            &triple,
            &host_cpu_name(),
            &host_cpu_features(),
            CodeGenerationOptimizationLevel::Aggressive,
            RelocationMode::PositionIndependentCode,
            CodeModel::Default,
        ))
    }

    /// This machine's target.
    #[must_use]
    pub fn target(&self) -> Target {
        // SAFETY: the machine is live.
        Target { raw: unsafe { raw::LLVMGetTargetMachineTarget(self.raw) } }
    }

    /// This machine's triple.
    #[must_use]
    pub fn triple(&self) -> String {
        // SAFETY: the machine is live, and LLVM allocates the text for the caller to free.
        from_owned_message(unsafe { raw::LLVMGetTargetMachineTriple(self.raw) })
    }

    /// This machine's processor.
    #[must_use]
    pub fn cpu(&self) -> String {
        // SAFETY: the machine is live, and LLVM allocates the text for the caller to free.
        from_owned_message(unsafe { raw::LLVMGetTargetMachineCPU(self.raw) })
    }

    /// This machine's features.
    #[must_use]
    pub fn features(&self) -> String {
        // SAFETY: the machine is live, and LLVM allocates the text for the caller to free.
        from_owned_message(unsafe { raw::LLVMGetTargetMachineFeatureString(self.raw) })
    }

    /// This machine's data layout.
    #[must_use]
    pub fn data_layout(&self) -> TargetData<'static> {
        // SAFETY: the machine is live, and LLVM makes a new layout for the caller to own.
        TargetData { raw: unsafe { raw::LLVMCreateTargetDataLayout(self.raw) }, owned: true, marker: PhantomData }
    }

    /// Sets whether assembly it writes has comments.
    pub fn set_assembly_verbosity(&mut self, verbose: bool) {
        // SAFETY: the machine is live.
        unsafe { raw::LLVMSetTargetMachineAsmVerbosity(self.raw, i32::from(verbose)) };
    }

    /// Sets whether it uses the fast instruction selector.
    pub fn set_fast_instruction_selection(&mut self, enable: bool) {
        // SAFETY: the machine is live.
        unsafe { raw::LLVMSetTargetMachineFastISel(self.raw, i32::from(enable)) };
    }

    /// Sets whether it uses `GlobalISel`.
    pub fn set_global_instruction_selection(&mut self, enable: bool) {
        // SAFETY: the machine is live.
        unsafe { raw::LLVMSetTargetMachineGlobalISel(self.raw, i32::from(enable)) };
    }

    /// Sets what `GlobalISel` does when it can't handle something.
    pub fn set_global_instruction_selection_abort(&mut self, mode: GlobalInstructionSelectionAbortMode) {
        // SAFETY: the machine is live.
        unsafe { raw::LLVMSetTargetMachineGlobalISelAbort(self.raw, mode.to_raw()) };
    }

    /// Sets whether it outlines repeated machine code.
    pub fn set_machine_outliner(&mut self, enable: bool) {
        // SAFETY: the machine is live.
        unsafe { raw::LLVMSetTargetMachineMachineOutliner(self.raw, i32::from(enable)) };
    }

    // trace: D11
    /// Writes this module's code to a file.
    ///
    /// Code generation runs passes that can change the module, so this needs exclusive access, as running passes does.
    ///
    /// # Errors
    /// What stopped the code being made or written, or the verifier's report if the module is invalid, since LLVM would crash on
    /// it.
    pub fn emit_to_file(
        &self,
        module: &mut Module<'_>,
        path: &Path,
        file_type: CodeGenerationFileType,
    ) -> Result<(), Error> {
        let path = c_path(path)?;
        module.verify_before("emit_to_file")?;
        module.bump_generation();
        let mut message = std::ptr::null_mut();
        // SAFETY: both are live, the module exclusively; `LLVMTargetMachineEmitToFile` takes a non-const path only by
        // old habit; it writes a message for the caller to free if it fails.
        let failed = unsafe {
            raw::LLVMTargetMachineEmitToFile(self.raw, module.raw, path.as_ptr(), file_type.to_raw(), &raw mut message)
        };
        if failed != 0 {
            return Err(Error::from_owned_message(message));
        }
        Ok(())
    }

    // trace: D11
    /// This module's code, in memory.
    ///
    /// Code generation runs passes that can change the module, so this needs exclusive access, as running passes does.
    ///
    /// # Errors
    /// What stopped the code being made, or the verifier's report if the module is invalid, since LLVM would crash on it.
    pub fn emit_to_memory_buffer(
        &self,
        module: &mut Module<'_>,
        file_type: CodeGenerationFileType,
    ) -> Result<MemoryBuffer<'static>, Error> {
        module.verify_before("emit_to_memory_buffer")?;
        module.bump_generation();
        let mut message = std::ptr::null_mut();
        let mut buffer = std::ptr::null_mut();
        // SAFETY: both are live, the module exclusively; LLVM writes a buffer for the caller to own, or a message for the
        // caller to free.
        let failed = unsafe {
            raw::LLVMTargetMachineEmitToMemoryBuffer(
                self.raw,
                module.raw,
                file_type.to_raw(),
                &raw mut message,
                &raw mut buffer,
            )
        };
        if failed != 0 {
            return Err(Error::from_owned_message(message));
        }
        // SAFETY: LLVM just made this buffer, and it owns its bytes.
        Ok(unsafe { MemoryBuffer::from_raw(buffer) })
    }
}

/// A data layout: how big things are and how they are aligned on a target.
pub struct TargetData<'data> {
    raw: raw::LLVMTargetDataRef,
    owned: bool,
    marker: PhantomData<&'data ()>,
}

impl Drop for TargetData<'_> {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: this wrapper owns the layout.
            unsafe { raw::LLVMDisposeTargetData(self.raw) };
        }
    }
}

impl TargetData<'_> {
    /// Wraps a layout something else owns for `'data`.
    pub(crate) fn borrowed(raw: raw::LLVMTargetDataRef) -> Self {
        TargetData { raw, owned: false, marker: PhantomData }
    }
}

impl TargetData<'static> {
    /// A data layout from its text.
    ///
    /// # Errors
    /// If LLVM can't parse it.
    pub fn new(text: &str) -> Result<Self, Error> {
        let Ok(text) = std::ffi::CString::new(text) else {
            return Err(Error::new("a data layout has no NUL bytes in it"));
        };
        // SAFETY: only reads the text.
        if !unsafe { shim::lyftar_data_layout_is_valid(text.as_ptr()) } {
            return Err(Error::new(format!("LLVM can't parse the data layout {text:?}")));
        }
        // SAFETY: the text was just checked, and LLVM makes a new layout for the caller to own.
        Ok(TargetData { raw: unsafe { raw::LLVMCreateTargetData(text.as_ptr()) }, owned: true, marker: PhantomData })
    }
}

impl TargetData<'_> {
    /// This layout's text.
    #[must_use]
    pub fn to_text(&self) -> String {
        // SAFETY: the layout is live, and LLVM allocates the text for the caller to free.
        from_owned_message(unsafe { raw::LLVMCopyStringRepOfTargetData(self.raw) })
    }

    /// The order of a value's bytes in memory.
    #[must_use]
    pub fn byte_order(&self) -> ByteOrdering {
        // SAFETY: the layout is live.
        ByteOrdering::from_raw(unsafe { raw::LLVMByteOrder(self.raw) })
    }

    /// A pointer's size in the default address space.
    #[must_use]
    pub fn pointer_size_in_bytes(&self) -> u32 {
        // SAFETY: the layout is live.
        unsafe { raw::LLVMPointerSize(self.raw) }
    }

    /// A pointer's size in an address space.
    #[must_use]
    pub fn pointer_size_in_bytes_for_address_space(&self, address_space: u32) -> u32 {
        // SAFETY: the layout is live.
        unsafe { raw::LLVMPointerSizeForAS(self.raw, address_space) }
    }

    /// The integer type as wide as a pointer in the default address space.
    #[must_use]
    pub fn integer_pointer_type<'context>(&self, context: Context<'context>) -> Type<'context, Integer> {
        // SAFETY: both are live; the type belongs to the context.
        unsafe { Type::from_raw(raw::LLVMIntPtrTypeInContext(context.raw, self.raw)) }
    }

    /// The integer type as wide as a pointer in an address space.
    #[must_use]
    pub fn integer_pointer_type_for_address_space<'context>(
        &self,
        context: Context<'context>,
        address_space: u32,
    ) -> Type<'context, Integer> {
        // SAFETY: both are live; the type belongs to the context.
        unsafe { Type::from_raw(raw::LLVMIntPtrTypeForASInContext(context.raw, self.raw, address_space)) }
    }

    #[track_caller]
    fn check_sized<T>(operation: &str, checked: Type<'_, T>) {
        if !checked.is_sized() {
            bad_call(format_args!("{operation}: {} has no size", checked.describe()));
        }
    }

    /// A type's size.
    ///
    /// # Panics
    /// If the type has no size (D3).
    #[must_use]
    #[track_caller]
    pub fn size_of_type_in_bits<T>(&self, of: Type<'_, T>) -> u64 {
        Self::check_sized("size_of_type_in_bits", of);
        // SAFETY: both are live, and the type was just checked.
        unsafe { raw::LLVMSizeOfTypeInBits(self.raw, of.raw) }
    }

    /// How many bytes storing a value of a type can touch.
    ///
    /// # Panics
    /// If the type has no size (D3).
    #[must_use]
    #[track_caller]
    pub fn store_size_in_bytes<T>(&self, of: Type<'_, T>) -> u64 {
        Self::check_sized("store_size_in_bytes", of);
        // SAFETY: both are live, and the type was just checked.
        unsafe { raw::LLVMStoreSizeOfType(self.raw, of.raw) }
    }

    /// How many bytes a value of a type takes up, with its padding.
    ///
    /// # Panics
    /// If the type has no size (D3).
    #[must_use]
    #[track_caller]
    pub fn allocation_size_in_bytes<T>(&self, of: Type<'_, T>) -> u64 {
        Self::check_sized("allocation_size_in_bytes", of);
        // SAFETY: both are live, and the type was just checked.
        unsafe { raw::LLVMABISizeOfType(self.raw, of.raw) }
    }

    /// A type's required alignment.
    ///
    /// # Panics
    /// If the type has no size (D3).
    #[must_use]
    #[track_caller]
    pub fn abi_alignment_in_bytes<T>(&self, of: Type<'_, T>) -> u32 {
        Self::check_sized("abi_alignment_in_bytes", of);
        // SAFETY: both are live, and the type was just checked.
        unsafe { raw::LLVMABIAlignmentOfType(self.raw, of.raw) }
    }

    /// A type's alignment on the call frame.
    ///
    /// # Panics
    /// If the type has no size (D3).
    #[must_use]
    #[track_caller]
    pub fn call_frame_alignment_in_bytes<T>(&self, of: Type<'_, T>) -> u32 {
        Self::check_sized("call_frame_alignment_in_bytes", of);
        // SAFETY: both are live, and the type was just checked.
        unsafe { raw::LLVMCallFrameAlignmentOfType(self.raw, of.raw) }
    }

    /// A type's preferred alignment.
    ///
    /// # Panics
    /// If the type has no size (D3).
    #[must_use]
    #[track_caller]
    pub fn preferred_alignment_in_bytes<T>(&self, of: Type<'_, T>) -> u32 {
        Self::check_sized("preferred_alignment_in_bytes", of);
        // SAFETY: both are live, and the type was just checked.
        unsafe { raw::LLVMPreferredAlignmentOfType(self.raw, of.raw) }
    }

    /// A global's preferred alignment.
    #[must_use]
    pub fn preferred_alignment_of_global_in_bytes(&self, global: Value<'_, '_, GlobalVariable>) -> u32 {
        // SAFETY: both are live, and the marker guarantees a global variable.
        unsafe { raw::LLVMPreferredAlignmentOfGlobal(self.raw, global.raw) }
    }

    #[track_caller]
    fn check_struct(operation: &str, checked: Type<'_>) {
        if checked.raw_kind() != raw::LLVMTypeKind::LLVMStructTypeKind || !checked.is_sized() {
            bad_call(format_args!("{operation}: {} is not a sized struct", checked.describe()));
        }
    }

    /// The index of the struct element holding the byte at `offset_in_bytes`.
    ///
    /// # Panics
    /// If the type isn't a sized struct (D3).
    #[must_use]
    #[track_caller]
    pub fn element_at_offset(&self, struct_type: Type<'_>, offset_in_bytes: u64) -> u32 {
        Self::check_struct("element_at_offset", struct_type);
        // SAFETY: just checked, and LLVM's search holds for any offset.
        unsafe { raw::LLVMElementAtOffset(self.raw, struct_type.raw, offset_in_bytes) }
    }

    /// Where a struct's element starts.
    ///
    /// # Panics
    /// If the type isn't a sized struct with such an element (D3).
    #[must_use]
    #[track_caller]
    pub fn offset_of_element_in_bytes(&self, struct_type: Type<'_>, element: u32) -> u64 {
        Self::check_struct("offset_of_element_in_bytes", struct_type);
        let count = struct_type.struct_element_count();
        if !checks::index_in_bounds(element, count) {
            bad_call(format_args!("offset_of_element_in_bytes: {} has no element {element}", struct_type.describe()));
        }
        // SAFETY: just checked.
        unsafe { raw::LLVMOffsetOfElement(self.raw, struct_type.raw, element) }
    }
}

impl Module<'_> {
    /// This module's data layout.
    #[must_use]
    pub fn target_data(&self) -> TargetData<'_> {
        // SAFETY: the module is live, and owns the layout for as long as the borrow lasts.
        TargetData { raw: unsafe { raw::LLVMGetModuleDataLayout(self.raw) }, owned: false, marker: PhantomData }
    }

    /// Sets this module's data layout.
    pub fn set_target_data(&self, data_layout: &TargetData<'_>) {
        // SAFETY: both are live, and LLVM copies it.
        unsafe { raw::LLVMSetModuleDataLayout(self.raw, data_layout.raw) };
    }

    /// Sets this module's target triple and data layout to a machine's.
    pub fn set_target_machine(&self, machine: &TargetMachine) {
        self.set_target(&machine.triple());
        self.set_target_data(&machine.data_layout());
    }
}
