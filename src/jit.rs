//! Running generated code: the execution engine, and ORC's LLJIT.
//!
//! A long-lived JIT takes a fresh context per compile (D10): [`Jit::add_module`] runs a closure with a new context, which goes
//! with the module into the JIT. Calling what the JIT made means casting an address to a function, which is `unsafe`.

use std::ffi::{CString, c_char, c_void};
use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::context::Context;
use crate::error::Error;
use crate::memory_buffer::MemoryBuffer;
use crate::module::{Module, OwnedModule};
use crate::raw;
use crate::shim;
use crate::target::{CodeModel, TargetData, TargetMachine};
use crate::text::{c_string, from_borrowed, from_owned_message};
use crate::types::{Float, Integer, Type};
use crate::values::{Function, GlobalValueMarker, Value};

/// A value passed to or from a function run by [`ExecutionEngine::run_function`].
pub struct GenericValue {
    raw: raw::LLVMGenericValueRef,
}

impl Drop for GenericValue {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the value.
        unsafe { raw::LLVMDisposeGenericValue(self.raw) };
    }
}

impl GenericValue {
    /// An integer.
    #[must_use]
    pub fn from_integer<W>(integer_type: Type<'_, Integer<W>>, value: u64, is_signed: bool) -> Self {
        // SAFETY: `Integer` guarantees an integer type, and LLVM makes a new value for the caller to own.
        GenericValue { raw: unsafe { raw::LLVMCreateGenericValueOfInt(integer_type.raw, value, i32::from(is_signed)) } }
    }

    /// A pointer.
    // Nothing reads through it here: LLVM holds it as a number until generated code uses it, which is `unsafe` to run.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    #[must_use]
    pub fn from_pointer(pointer: *mut c_void) -> Self {
        // SAFETY: the pointer is only held as a number; LLVM makes a new value for the caller to own.
        GenericValue { raw: unsafe { raw::LLVMCreateGenericValueOfPointer(pointer) } }
    }

    /// A float of a format LLVM keeps as `float` or `double` here.
    ///
    /// # Panics
    /// If it is any other format, which LLVM's C API can't hold (D3).
    #[must_use]
    #[track_caller]
    pub fn from_float<F>(float_type: Type<'_, Float<F>>, value: f64) -> Self {
        use raw::LLVMTypeKind as K;
        if !matches!(float_type.raw_kind(), K::LLVMFloatTypeKind | K::LLVMDoubleTypeKind) {
            bad_call(format_args!("from_float: only `float` and `double`, not {}", float_type.describe()));
        }
        // SAFETY: just checked; LLVM makes a new value for the caller to own.
        GenericValue { raw: unsafe { raw::LLVMCreateGenericValueOfFloat(float_type.raw, value) } }
    }

    /// The width of this integer.
    #[must_use]
    pub fn integer_width_in_bits(&self) -> u32 {
        // SAFETY: the value is live; a value that isn't an integer holds a zero-width number here.
        unsafe { raw::LLVMGenericValueIntWidth(self.raw) }
    }

    /// This integer's value.
    #[must_use]
    pub fn to_integer(&self, is_signed: bool) -> u64 {
        // SAFETY: the value is live.
        unsafe { raw::LLVMGenericValueToInt(self.raw, i32::from(is_signed)) }
    }

    /// This pointer's value.
    #[must_use]
    pub fn to_pointer(&self) -> *mut c_void {
        // SAFETY: the value is live.
        unsafe { raw::LLVMGenericValueToPointer(self.raw) }
    }

    /// This float's value.
    ///
    /// # Panics
    /// If the type is any but `float` or `double`, which LLVM's C API can't hold (D3).
    #[must_use]
    #[track_caller]
    pub fn to_float<F>(&self, float_type: Type<'_, Float<F>>) -> f64 {
        use raw::LLVMTypeKind as K;
        if !matches!(float_type.raw_kind(), K::LLVMFloatTypeKind | K::LLVMDoubleTypeKind) {
            bad_call(format_args!("to_float: only `float` and `double`, not {}", float_type.describe()));
        }
        // SAFETY: just checked, and the value is live.
        unsafe { raw::LLVMGenericValueToFloat(float_type.raw, self.raw) }
    }
}

/// An execution engine: it owns its modules, and makes or interprets their code when asked.
pub struct ExecutionEngine<'context, 'borrow> {
    raw: raw::LLVMExecutionEngineRef,
    modules: Vec<raw::LLVMModuleRef>,
    stack_maps: *mut c_void,
    // trace: D42
    // What it gives back reads a module, so it borrows the context as `Context::module` does, and no pass can run meanwhile.
    marker: PhantomData<&'borrow Context<'context>>,
}

impl Drop for ExecutionEngine<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the engine, which owns its modules, and owns the capture, which goes once the engine has.
        unsafe {
            raw::LLVMDisposeExecutionEngine(self.raw);
            shim::lyftar_stack_maps_dispose(self.stack_maps);
        }
    }
}

/// Copies each section out of a capture.
///
/// # Safety
/// `captured` has to be a live capture, whose JIT or engine is live.
unsafe fn copy_stack_map_sections(captured: *mut c_void) -> Vec<Vec<u8>> {
    unsafe extern "C" fn push(state: *mut c_void, bytes: *const u8, size_in_bytes: usize) {
        // SAFETY: `state` is the vector below, for this call, and the shim gives this many readable bytes. Nothing here panics
        // across C++: a push that can't allocate aborts.
        unsafe {
            let sections = &mut *state.cast::<Vec<Vec<u8>>>();
            sections.push(std::slice::from_raw_parts(bytes, size_in_bytes).to_vec());
        }
    }
    let mut sections: Vec<Vec<u8>> = Vec::new();
    // SAFETY: the caller promises a live capture, and `push` only uses the vector, which outlives the call.
    unsafe { shim::lyftar_stack_maps_visit(captured, push, (&raw mut sections).cast()) };
    sections
}

/// How to set up an engine from [`ExecutionEngine::with_mcjit_options`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct McjitOptions {
    /// How hard the code generator tries, from 0 to 3.
    pub optimization_level: u32,
    /// What the code assumes about how far apart things are.
    pub code_model: CodeModel,
    /// Keeps the frame pointer.
    pub no_frame_pointer_elimination: bool,
    /// Uses the fast instruction selector.
    pub enable_fast_instruction_selection: bool,
}

impl Default for McjitOptions {
    fn default() -> Self {
        McjitOptions {
            optimization_level: 0,
            code_model: CodeModel::JitDefault,
            no_frame_pointer_elimination: false,
            enable_fast_instruction_selection: false,
        }
    }
}

impl<'context, 'borrow> ExecutionEngine<'context, 'borrow> {
    /// The raw engine.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(&self) -> raw::LLVMExecutionEngineRef {
        self.raw
    }

    fn from_module(
        _context: &'borrow Context<'context>,
        module: OwnedModule<'context>,
        create: impl FnOnce(raw::LLVMModuleRef, *mut raw::LLVMExecutionEngineRef, *mut *mut c_char) -> i32,
    ) -> Result<Self, Error> {
        crate::target::initialize_native_target()?;
        module.module.verify_before("ExecutionEngine")?;
        module.module.check_can_make_code("ExecutionEngine", &crate::target::default_target_triple())?;
        let raw_module = module.into_raw();
        let mut engine = std::ptr::null_mut();
        let mut message = std::ptr::null_mut();
        let failed = create(raw_module, &raw mut engine, &raw mut message);
        if failed != 0 {
            // SAFETY: the engine wasn't made, so the module is still the caller's to free.
            unsafe { raw::LLVMDisposeModule(raw_module) };
            return Err(Error::from_owned_message(message));
        }
        // SAFETY: LLVM just made the engine, and this wrapper owns the capture from here on.
        let stack_maps = unsafe { shim::lyftar_execution_engine_capture_stack_maps(engine) };
        Ok(ExecutionEngine { raw: engine, modules: vec![raw_module], stack_maps, marker: PhantomData })
    }

    /// An engine for this module: a JIT if this machine has one, and otherwise an interpreter.
    ///
    /// # Errors
    /// What stopped it being made, or the verifier's report if the module is invalid, since LLVM would crash on it.
    pub fn new(context: &'borrow Context<'context>, module: OwnedModule<'context>) -> Result<Self, Error> {
        Self::from_module(context, module, |raw_module, engine, message| {
            // SAFETY: the pointers have room for what LLVM writes, and on success the engine owns the module.
            unsafe { raw::LLVMLinkInMCJIT() };
            // SAFETY: the same.
            unsafe { raw::LLVMCreateExecutionEngineForModule(engine, raw_module, message) }
        })
    }

    /// An interpreter for this module.
    ///
    /// # Errors
    /// What stopped it being made, or the verifier's report if the module is invalid, since LLVM would crash on it.
    pub fn interpreter(context: &'borrow Context<'context>, module: OwnedModule<'context>) -> Result<Self, Error> {
        Self::from_module(context, module, |raw_module, engine, message| {
            // SAFETY: the pointers have room for what LLVM writes, and on success the engine owns the module.
            unsafe { raw::LLVMLinkInInterpreter() };
            // SAFETY: the same.
            unsafe { raw::LLVMCreateInterpreterForModule(engine, raw_module, message) }
        })
    }

    /// A JIT for this module, optimizing to a level from 0 to 3.
    ///
    /// # Errors
    /// What stopped it being made, or the verifier's report if the module is invalid, since LLVM would crash on it.
    pub fn jit_compiler(
        context: &'borrow Context<'context>,
        module: OwnedModule<'context>,
        optimization_level: u32,
    ) -> Result<Self, Error> {
        Self::from_module(context, module, |raw_module, engine, message| {
            // SAFETY: the pointers have room for what LLVM writes, and on success the engine owns the module.
            unsafe { raw::LLVMLinkInMCJIT() };
            // SAFETY: the same.
            unsafe { raw::LLVMCreateJITCompilerForModule(engine, raw_module, optimization_level, message) }
        })
    }

    /// A JIT for this module, set up by options.
    ///
    /// # Errors
    /// What stopped it being made, or the verifier's report if the module is invalid, since LLVM would crash on it.
    pub fn with_mcjit_options(
        context: &'borrow Context<'context>,
        module: OwnedModule<'context>,
        options: McjitOptions,
    ) -> Result<Self, Error> {
        Self::from_module(context, module, |raw_module, engine, message| {
            // SAFETY: LLVM fills the struct with its defaults first, the size says which fields it may touch, and on
            // success the engine owns the module. There is no custom memory manager.
            unsafe {
                raw::LLVMLinkInMCJIT();
                let mut raw_options = std::mem::zeroed::<raw::LLVMMCJITCompilerOptions>();
                let size = size_of::<raw::LLVMMCJITCompilerOptions>();
                raw::LLVMInitializeMCJITCompilerOptions(&raw mut raw_options, size);
                raw_options.OptLevel = options.optimization_level;
                raw_options.CodeModel = options.code_model.to_raw();
                raw_options.NoFramePointerElim = i32::from(options.no_frame_pointer_elimination);
                raw_options.EnableFastISel = i32::from(options.enable_fast_instruction_selection);
                raw::LLVMCreateMCJITCompilerForModule(engine, raw_module, &raw mut raw_options, size, message)
            }
        })
    }

    // trace: D45
    /// The stack map section of each object this engine has made, with its addresses relocated, for `lyftar-stack-maps` to read.
    /// This makes and relocates any code it hasn't yet. The interpreter makes none.
    #[must_use]
    pub fn stack_map_sections(&self) -> Vec<Vec<u8>> {
        // SAFETY: the engine is live, and its capture lasts as long as it does.
        unsafe {
            shim::lyftar_execution_engine_finish_code(self.raw);
            copy_stack_map_sections(self.stack_maps)
        }
    }

    /// Gives this engine another module.
    ///
    /// # Panics
    /// If the verifier refuses the module, since LLVM would crash on it (D3).
    #[track_caller]
    pub fn add_module(&mut self, module: OwnedModule<'context>) {
        if let Err(report) = module.module.verify_before("add_module") {
            bad_call(format_args!("{}", report.message()));
        }
        let raw_module = module.into_raw();
        // SAFETY: the engine is live, and owns the module from here on.
        unsafe { raw::LLVMAddModule(self.raw, raw_module) };
        self.modules.push(raw_module);
    }

    /// Takes back the module with this name.
    ///
    /// # Errors
    /// If LLVM couldn't remove it.
    ///
    /// # Panics
    /// If this engine has no module with that name (D3).
    #[track_caller]
    pub fn remove_module(&mut self, identifier: &str) -> Result<OwnedModule<'context>, Error> {
        let position = self.modules.iter().position(|found| {
            let mut length = 0;
            // SAFETY: the engine keeps its modules live; the text is copied right away.
            let name = unsafe { raw::LLVMGetModuleIdentifier(*found, &raw mut length) };
            crate::text::from_pointer_and_length(name, length) == identifier
        });
        let Some(position) = position else {
            bad_call(format_args!("remove_module: this engine has no module {identifier}"))
        };
        let raw_module = self.modules.remove(position);
        let mut out = std::ptr::null_mut();
        let mut message = std::ptr::null_mut();
        // SAFETY: the engine holds this module; LLVM gives it back, or a message for the caller to free.
        let failed = unsafe { raw::LLVMRemoveModule(self.raw, raw_module, &raw mut out, &raw mut message) };
        if failed != 0 {
            self.modules.push(raw_module);
            return Err(Error::from_owned_message(message));
        }
        // SAFETY: LLVM just gave the module back, for the caller to own.
        Ok(unsafe { OwnedModule::from_raw(out) })
    }

    /// The function with this name in any of this engine's modules.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn find_function(&self, name: &str) -> Option<Value<'context, '_, Function>> {
        let name = c_string(name, "find_function");
        let mut found = std::ptr::null_mut();
        // SAFETY: a valid C string; LLVM writes the function it finds, live while the engine holds its module.
        let missing = unsafe { raw::LLVMFindFunction(self.raw, name.as_ptr(), &raw mut found) };
        // SAFETY: just found, in a module this engine owns, so it lasts as long as this borrow.
        (missing == 0).then(|| unsafe { Value::from_raw(found) })
    }

    /// Runs the modules' static constructors.
    ///
    /// # Safety
    /// This runs generated code, which can do anything.
    pub unsafe fn run_static_constructors(&self) {
        // SAFETY: the caller takes responsibility for the generated code.
        unsafe { raw::LLVMRunStaticConstructors(self.raw) };
    }

    /// Runs the modules' static destructors.
    ///
    /// # Safety
    /// This runs generated code, which can do anything.
    pub unsafe fn run_static_destructors(&self) {
        // SAFETY: the caller takes responsibility for the generated code.
        unsafe { raw::LLVMRunStaticDestructors(self.raw) };
    }

    /// Runs a function as `main`.
    ///
    /// # Safety
    /// This runs generated code, which can do anything.
    ///
    /// # Panics
    /// If an argument has a NUL byte inside it (D3).
    #[track_caller]
    pub unsafe fn run_function_as_main(
        &self,
        function: Value<'context, '_, Function>,
        arguments: &[&str],
        environment: &[&str],
    ) -> i32 {
        let to_c_strings = |items: &[&str]| -> Vec<CString> {
            items.iter().map(|item| c_string(item, "run_function_as_main")).collect()
        };
        let arguments = to_c_strings(arguments);
        let environment = to_c_strings(environment);
        let argument_pointers: Vec<*const c_char> = arguments.iter().map(|argument| argument.as_ptr()).collect();
        let mut environment_pointers: Vec<*const c_char> = environment.iter().map(|item| item.as_ptr()).collect();
        environment_pointers.push(std::ptr::null());
        let count = crate::types::length_as_u32(arguments.len(), "run_function_as_main");
        // SAFETY: the lists last for the call, and the environment ends in a null, as LLVM wants. The caller takes
        // responsibility for the generated code.
        unsafe {
            raw::LLVMRunFunctionAsMain(
                self.raw,
                function.raw,
                count,
                argument_pointers.as_ptr(),
                environment_pointers.as_ptr(),
            )
        }
    }

    /// Runs a function with these arguments. LLVM's JIT only runs some signatures this way, and stops the process on others,
    /// where an address from [`ExecutionEngine::function_address`] always works.
    ///
    /// # Safety
    /// This runs generated code, which can do anything.
    #[track_caller]
    pub unsafe fn run_function(
        &self,
        function: Value<'context, '_, Function>,
        arguments: &[&GenericValue],
    ) -> GenericValue {
        let mut raw_arguments: Vec<raw::LLVMGenericValueRef> = arguments.iter().map(|argument| argument.raw).collect();
        let count = crate::types::length_as_u32(arguments.len(), "run_function");
        // SAFETY: LLVM only reads the arguments, and makes a new value for the caller to own. The caller takes
        // responsibility for the generated code.
        GenericValue { raw: unsafe { raw::LLVMRunFunction(self.raw, function.raw, count, raw_arguments.as_mut_ptr()) } }
    }

    /// Tells the JIT where a global lives, for a declaration with no body.
    // Nothing reads through it here: LLVM holds it as a number until generated code uses it, which is `unsafe` to run.
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    pub fn add_global_mapping<T: GlobalValueMarker>(&self, global: Value<'context, '_, T>, address: *mut c_void) {
        // SAFETY: both are live, and the address is only held until generated code uses it, which is `unsafe` to run.
        unsafe { raw::LLVMAddGlobalMapping(self.raw, global.raw, address) };
    }

    /// A global's address, making code for it if needed.
    #[must_use]
    pub fn pointer_to_global<T: GlobalValueMarker>(&self, global: Value<'context, '_, T>) -> *mut c_void {
        // SAFETY: both are live; what comes back is only a number until it is used, which is `unsafe`.
        unsafe { raw::LLVMGetPointerToGlobal(self.raw, global.raw) }
    }

    /// The address of the global with this name, or zero, making code for it if needed.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn global_value_address(&self, name: &str) -> u64 {
        let name = c_string(name, "global_value_address");
        // SAFETY: a valid C string for the call.
        unsafe { raw::LLVMGetGlobalValueAddress(self.raw, name.as_ptr()) }
    }

    /// The address of the function with this name, or zero, making code for it if needed.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn function_address(&self, name: &str) -> u64 {
        let name = c_string(name, "function_address");
        // SAFETY: a valid C string for the call.
        unsafe { raw::LLVMGetFunctionAddress(self.raw, name.as_ptr()) }
    }

    /// The error the JIT last hit, if any.
    #[must_use]
    pub fn error_message(&self) -> Option<String> {
        let mut message = std::ptr::null_mut();
        // SAFETY: LLVM writes a message for the caller to free, if there is an error.
        let has_error = unsafe { raw::LLVMExecutionEngineGetErrMsg(self.raw, &raw mut message) };
        (has_error != 0).then(|| from_owned_message(message))
    }

    /// The data layout this engine uses.
    #[must_use]
    pub fn target_data(&self) -> TargetData<'_> {
        // SAFETY: the engine owns the layout for as long as this borrow lasts.
        TargetData::borrowed(unsafe { raw::LLVMGetExecutionEngineTargetData(self.raw) })
    }

    /// The triple of the target machine this engine uses.
    #[must_use]
    pub fn target_machine_triple(&self) -> String {
        // SAFETY: the engine owns the machine; LLVM allocates the text for the caller to free.
        from_owned_message(unsafe {
            raw::LLVMGetTargetMachineTriple(raw::LLVMGetExecutionEngineTargetMachine(self.raw))
        })
    }
}

/// A listener for what the JIT makes, for debuggers and profilers.
pub struct JitEventListener {
    raw: raw::LLVMJITEventListenerRef,
}

impl JitEventListener {
    fn from_raw_option(raw: raw::LLVMJITEventListenerRef) -> Option<Self> {
        (!raw.is_null()).then_some(JitEventListener { raw })
    }

    /// Registers code with a debugger that follows GDB's protocol.
    #[must_use]
    pub fn gdb_registration() -> Self {
        // SAFETY: takes nothing, and LLVM owns the listener, which lasts for good.
        JitEventListener { raw: unsafe { raw::LLVMCreateGDBRegistrationListener() } }
    }

    /// Intel's profiler, if LLVM was built with it.
    #[must_use]
    pub fn intel() -> Option<Self> {
        // SAFETY: takes nothing; a null means LLVM wasn't built with it.
        Self::from_raw_option(unsafe { raw::LLVMCreateIntelJITEventListener() })
    }

    /// `OProfile`, if LLVM was built with it.
    #[must_use]
    pub fn oprofile() -> Option<Self> {
        // SAFETY: takes nothing; a null means LLVM wasn't built with it.
        Self::from_raw_option(unsafe { raw::LLVMCreateOProfileJITEventListener() })
    }

    /// Linux's `perf`, if LLVM was built with it.
    #[must_use]
    pub fn perf() -> Option<Self> {
        // SAFETY: takes nothing; a null means LLVM wasn't built with it.
        Self::from_raw_option(unsafe { raw::LLVMCreatePerfJITEventListener() })
    }
}

/// A name in ORC's string pool.
pub struct SymbolStringPoolEntry {
    raw: raw::LLVMOrcSymbolStringPoolEntryRef,
}

impl Drop for SymbolStringPoolEntry {
    fn drop(&mut self) {
        // SAFETY: this wrapper holds a reference to the entry.
        unsafe { raw::LLVMOrcReleaseSymbolStringPoolEntry(self.raw) };
    }
}

impl Clone for SymbolStringPoolEntry {
    fn clone(&self) -> Self {
        // SAFETY: the entry is live, and the new wrapper holds the new reference.
        unsafe { raw::LLVMOrcRetainSymbolStringPoolEntry(self.raw) };
        SymbolStringPoolEntry { raw: self.raw }
    }
}

impl SymbolStringPoolEntry {
    /// The name.
    #[must_use]
    pub fn as_string(&self) -> String {
        // SAFETY: the entry is live; the text is copied right away.
        from_borrowed(unsafe { raw::LLVMOrcSymbolStringPoolEntryStr(self.raw) })
    }
}

/// How ORC treats a symbol.
// One flag each is how LLVM says these.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitSymbolFlags {
    /// Visible outside its JIT dynamic library.
    pub exported: bool,
    /// Can be overridden by a strong definition.
    pub weak: bool,
    /// Is a function.
    pub callable: bool,
    /// Exists only for what materializing it does.
    pub materialization_side_effects_only: bool,
}

impl JitSymbolFlags {
    fn to_raw(self) -> raw::LLVMJITSymbolFlags {
        use raw::LLVMJITSymbolGenericFlags as F;
        let mut flags = 0;
        if self.exported {
            flags |= F::LLVMJITSymbolGenericFlagsExported.0;
        }
        if self.weak {
            flags |= F::LLVMJITSymbolGenericFlagsWeak.0;
        }
        if self.callable {
            flags |= F::LLVMJITSymbolGenericFlagsCallable.0;
        }
        if self.materialization_side_effects_only {
            flags |= F::LLVMJITSymbolGenericFlagsMaterializationSideEffectsOnly.0;
        }
        // The flags all fit in LLVM's byte.
        #[allow(clippy::cast_possible_truncation)]
        raw::LLVMJITSymbolFlags { GenericFlags: flags as u8, TargetFlags: 0 }
    }
}

/// A JIT dynamic library: a table of symbols in a [`Jit`].
#[derive(Clone, Copy)]
pub struct JitDylib<'jit> {
    raw: raw::LLVMOrcJITDylibRef,
    marker: PhantomData<&'jit ()>,
}

/// Part of a JIT dynamic library that can be removed by itself: everything added with the same tracker goes together.
pub struct ResourceTracker<'jit> {
    raw: raw::LLVMOrcResourceTrackerRef,
    marker: PhantomData<&'jit ()>,
}

impl Drop for ResourceTracker<'_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper holds a reference to the tracker.
        unsafe { raw::LLVMOrcReleaseResourceTracker(self.raw) };
    }
}

impl ResourceTracker<'_> {
    /// Removes everything added with this tracker. What was made may be in use, so it must not run again.
    ///
    /// # Errors
    /// What stopped it being removed.
    pub fn remove(&self) -> Result<(), Error> {
        // SAFETY: the tracker is live.
        Error::from_raw_result(unsafe { raw::LLVMOrcResourceTrackerRemove(self.raw) })
    }

    /// Moves what this tracker has to another.
    pub fn transfer_to(&self, destination: &ResourceTracker<'_>) {
        // SAFETY: both are live.
        unsafe { raw::LLVMOrcResourceTrackerTransferTo(self.raw, destination.raw) };
    }
}

/// Sets up a [`Jit`].
pub struct JitBuilder {
    raw: raw::LLVMOrcLLJITBuilderRef,
    runtime_dyld: bool,
}

impl Drop for JitBuilder {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: this wrapper owns the builder, as nothing has taken it.
            unsafe { raw::LLVMOrcDisposeLLJITBuilder(self.raw) };
        }
    }
}

impl Default for JitBuilder {
    fn default() -> Self {
        Self::new()
    }
}

unsafe extern "C" fn create_runtime_dyld_layer(
    _context: *mut c_void,
    session: raw::LLVMOrcExecutionSessionRef,
    _triple: *const c_char,
) -> raw::LLVMOrcObjectLayerRef {
    // SAFETY: LLVM gives a live session, and the layer it makes goes to LLJIT.
    unsafe { raw::LLVMOrcCreateRTDyldObjectLinkingLayerWithSectionMemoryManager(session) }
}

impl JitBuilder {
    /// The default setup.
    #[must_use]
    pub fn new() -> Self {
        // SAFETY: takes nothing, and the builder is owned by the wrapper from here on.
        JitBuilder { raw: unsafe { raw::LLVMOrcCreateLLJITBuilder() }, runtime_dyld: false }
    }

    /// Makes code for this machine, instead of for the computer this is running on.
    pub fn set_target_machine(&mut self, machine: TargetMachine) {
        // SAFETY: both are live; LLVM takes both the machine and the machine builder, which is made from it.
        unsafe {
            let raw_machine = machine.into_raw();
            let machine_builder = raw::LLVMOrcJITTargetMachineBuilderCreateFromTargetMachine(raw_machine);
            raw::LLVMOrcLLJITBuilderSetJITTargetMachineBuilder(self.raw, machine_builder);
        }
    }

    /// Makes code by a target triple.
    ///
    /// # Errors
    /// If LLVM can't describe this computer to start from.
    ///
    /// # Panics
    /// If the triple has a NUL byte inside it (D3).
    #[track_caller]
    pub fn set_target_triple(&mut self, triple: &str) -> Result<(), Error> {
        let triple = c_string(triple, "set_target_triple");
        let mut machine_builder = std::ptr::null_mut();
        // SAFETY: LLVM writes the machine builder when there is no error.
        Error::from_raw_result(unsafe { raw::LLVMOrcJITTargetMachineBuilderDetectHost(&raw mut machine_builder) })?;
        // SAFETY: the machine builder was just made, and goes to the JIT builder, which owns it from here on. The text LLVM
        // copied is freed after reading back that it was set.
        unsafe {
            raw::LLVMOrcJITTargetMachineBuilderSetTargetTriple(machine_builder, triple.as_ptr());
            drop(from_owned_message(raw::LLVMOrcJITTargetMachineBuilderGetTargetTriple(machine_builder)));
            raw::LLVMOrcLLJITBuilderSetJITTargetMachineBuilder(self.raw, machine_builder);
        }
        Ok(())
    }

    /// Links objects with `RuntimeDyld` rather than `JITLink`, which [`Jit::register_event_listener`] needs.
    pub fn use_runtime_dyld(&mut self) {
        // SAFETY: the builder is live, the function matches LLVM's signature, and it needs no state.
        unsafe {
            raw::LLVMOrcLLJITBuilderSetObjectLinkingLayerCreator(
                self.raw,
                Some(create_runtime_dyld_layer),
                std::ptr::null_mut(),
            );
        }
        self.runtime_dyld = true;
    }

    /// Makes the JIT.
    ///
    /// # Errors
    /// What stopped it being made.
    pub fn build(mut self) -> Result<Jit, Error> {
        crate::target::initialize_native_target()?;
        let mut jit = std::ptr::null_mut();
        let raw_builder = std::mem::replace(&mut self.raw, std::ptr::null_mut());
        // SAFETY: LLVM takes the builder in every case, and writes the JIT when there is no error.
        Error::from_raw_result(unsafe { raw::LLVMOrcCreateLLJIT(&raw mut jit, raw_builder) })?;
        // SAFETY: LLVM just made the JIT, and this wrapper owns the capture from here on.
        let stack_maps = unsafe { shim::lyftar_jit_capture_stack_maps(jit) };
        Ok(Jit { raw: jit, runtime_dyld: self.runtime_dyld, stack_maps })
    }
}

/// ORC's LLJIT: a long-lived JIT that makes code when a symbol is looked up (D10).
///
/// It doesn't belong to a context, so it can be kept across [`crate::with_context`]: each module it gets comes with its own
/// fresh context.
pub struct Jit {
    raw: raw::LLVMOrcLLJITRef,
    runtime_dyld: bool,
    stack_maps: *mut c_void,
}

impl Drop for Jit {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the JIT. An error from tearing it down has nowhere to go, so it is dropped.
        drop(Error::from_raw_result(unsafe { raw::LLVMOrcDisposeLLJIT(self.raw) }));
        // SAFETY: this wrapper owns the capture, whose JIT has just gone.
        unsafe { shim::lyftar_stack_maps_dispose(self.stack_maps) };
    }
}

/// Runs `make` with a fresh context, and wraps what it gives as ORC wants it. A module that doesn't verify would crash in the
/// code generator, so it comes back as an error instead.
fn thread_safe_module(
    make: impl for<'context> FnOnce(&mut Context<'context>) -> OwnedModule<'context>,
) -> Result<raw::LLVMOrcThreadSafeModuleRef, Error> {
    // SAFETY: creating a context has no preconditions. ORC's thread-safe context owns it from here on, and the module with
    // its context goes into the thread-safe module; the thread-safe context handle is dropped afterwards, as ORC's C API
    // documents.
    unsafe {
        let raw_context = raw::LLVMContextCreate();
        let thread_safe_context = raw::LLVMOrcCreateNewThreadSafeContextFromLLVMContext(raw_context);
        let module = make(&mut Context::from_raw(raw_context));
        let raw_module = module.into_raw();
        let verified = {
            let borrowed = Module::from_raw_borrowed(raw_module);
            borrowed
                .verify()
                .and_then(|()| borrowed.check_can_make_code("add_module", &crate::target::default_target_triple()))
        };
        if let Err(report) = verified {
            raw::LLVMDisposeModule(raw_module);
            raw::LLVMOrcDisposeThreadSafeContext(thread_safe_context);
            return Err(report);
        }
        let thread_safe = raw::LLVMOrcCreateNewThreadSafeModule(raw_module, thread_safe_context);
        raw::LLVMOrcDisposeThreadSafeContext(thread_safe_context);
        Ok(thread_safe)
    }
}

impl Jit {
    /// A JIT for the computer this is running on.
    ///
    /// # Errors
    /// What stopped it being made.
    pub fn new() -> Result<Jit, Error> {
        JitBuilder::new().build()
    }

    // trace: D45
    /// The stack map section of each object this JIT has linked so far, with its addresses relocated, for `lyftar-stack-maps` to
    /// read. Code is linked when a symbol is first looked up, so this has what has been looked up, and what a
    /// [`ResourceTracker`] removes goes from it.
    #[must_use]
    pub fn stack_map_sections(&self) -> Vec<Vec<u8>> {
        // SAFETY: the JIT is live, and its capture lasts as long as it does.
        unsafe { copy_stack_map_sections(self.stack_maps) }
    }

    /// The raw JIT.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(&self) -> raw::LLVMOrcLLJITRef {
        self.raw
    }

    /// The JIT dynamic library things go in by default.
    #[must_use]
    pub fn main_dylib(&self) -> JitDylib<'_> {
        // SAFETY: the JIT owns the library for as long as this borrow lasts.
        JitDylib { raw: unsafe { raw::LLVMOrcLLJITGetMainJITDylib(self.raw) }, marker: PhantomData }
    }

    /// A new, empty JIT dynamic library.
    ///
    /// # Errors
    /// If there already is one with this name.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn create_dylib(&self, name: &str) -> Result<JitDylib<'_>, Error> {
        let name = c_string(name, "create_dylib");
        let mut library = std::ptr::null_mut();
        // SAFETY: the session is live, the name is a valid C string, and LLVM writes the library when there is no error.
        Error::from_raw_result(unsafe {
            raw::LLVMOrcExecutionSessionCreateJITDylib(self.session(), &raw mut library, name.as_ptr())
        })?;
        Ok(JitDylib { raw: library, marker: PhantomData })
    }

    /// A new JIT dynamic library with no standard symbols added for this platform.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn create_bare_dylib(&self, name: &str) -> JitDylib<'_> {
        let name = c_string(name, "create_bare_dylib");
        // SAFETY: the session is live, and the name is a valid C string for the call.
        let library = unsafe { raw::LLVMOrcExecutionSessionCreateBareJITDylib(self.session(), name.as_ptr()) };
        JitDylib { raw: library, marker: PhantomData }
    }

    /// The JIT dynamic library with this name.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn dylib_by_name(&self, name: &str) -> Option<JitDylib<'_>> {
        let name = c_string(name, "dylib_by_name");
        // SAFETY: the session is live, and the name is a valid C string for the call.
        let library = unsafe { raw::LLVMOrcExecutionSessionGetJITDylibByName(self.session(), name.as_ptr()) };
        (!library.is_null()).then_some(JitDylib { raw: library, marker: PhantomData })
    }

    fn session(&self) -> raw::LLVMOrcExecutionSessionRef {
        // SAFETY: the JIT is live, and owns the session.
        unsafe { raw::LLVMOrcLLJITGetExecutionSession(self.raw) }
    }

    /// A name in the session's string pool.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn intern(&self, name: &str) -> SymbolStringPoolEntry {
        let name = c_string(name, "intern");
        // SAFETY: the session is live; the wrapper holds the reference LLVM gives back.
        SymbolStringPoolEntry { raw: unsafe { raw::LLVMOrcExecutionSessionIntern(self.session(), name.as_ptr()) } }
    }

    /// Drops the names in the string pool that nothing uses.
    pub fn clear_dead_symbol_names(&self) {
        // SAFETY: the session and its pool are live.
        unsafe {
            raw::LLVMOrcSymbolStringPoolClearDeadEntries(raw::LLVMOrcExecutionSessionGetSymbolStringPool(
                self.session(),
            ));
        };
    }

    /// A name mangled as this target's linker wants it, in the string pool.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn mangle_and_intern(&self, name: &str) -> SymbolStringPoolEntry {
        let name = c_string(name, "mangle_and_intern");
        // SAFETY: the JIT is live; the wrapper holds the reference LLVM gives back.
        SymbolStringPoolEntry { raw: unsafe { raw::LLVMOrcLLJITMangleAndIntern(self.raw, name.as_ptr()) } }
    }

    /// The target triple this JIT makes code for.
    #[must_use]
    pub fn triple(&self) -> String {
        // SAFETY: the JIT is live; the text is copied right away.
        from_borrowed(unsafe { raw::LLVMOrcLLJITGetTripleString(self.raw) })
    }

    /// The data layout this JIT makes code with.
    #[must_use]
    pub fn data_layout(&self) -> String {
        // SAFETY: the JIT is live; the text is copied right away.
        from_borrowed(unsafe { raw::LLVMOrcLLJITGetDataLayoutStr(self.raw) })
    }

    /// The prefix this target's linker puts on symbol names.
    #[must_use]
    pub fn global_prefix(&self) -> u8 {
        // SAFETY: the JIT is live.
        unsafe { raw::LLVMOrcLLJITGetGlobalPrefix(self.raw) }.cast_unsigned()
    }

    // trace: D10
    /// Compiles a module in its own fresh context: `make` builds it and hands it over. It runs nothing yet: code is made when
    /// a symbol is looked up.
    ///
    /// # Errors
    /// What stopped it being added, or the verifier's report.
    pub fn add_module(
        &self,
        library: JitDylib<'_>,
        make: impl for<'context> FnOnce(&mut Context<'context>) -> OwnedModule<'context>,
    ) -> Result<(), Error> {
        let thread_safe = thread_safe_module(make)?;
        // SAFETY: both are live, and LLVM takes the module in every case.
        Error::from_raw_result(unsafe { raw::LLVMOrcLLJITAddLLVMIRModule(self.raw, library.raw, thread_safe) })
    }

    // trace: D10
    /// As [`Jit::add_module`], under a resource tracker.
    ///
    /// # Errors
    /// What stopped it being added, or the verifier's report.
    pub fn add_module_with_tracker(
        &self,
        tracker: &ResourceTracker<'_>,
        make: impl for<'context> FnOnce(&mut Context<'context>) -> OwnedModule<'context>,
    ) -> Result<(), Error> {
        let thread_safe = thread_safe_module(make)?;
        // SAFETY: both are live, and LLVM takes the module in every case.
        Error::from_raw_result(unsafe { raw::LLVMOrcLLJITAddLLVMIRModuleWithRT(self.raw, tracker.raw, thread_safe) })
    }

    /// Adds an object file.
    ///
    /// # Errors
    /// What stopped it being added.
    pub fn add_object_file(&self, library: JitDylib<'_>, object: MemoryBuffer<'static>) -> Result<(), Error> {
        // SAFETY: both are live, and LLVM takes the buffer in every case.
        Error::from_raw_result(unsafe { raw::LLVMOrcLLJITAddObjectFile(self.raw, library.raw, object.into_raw()) })
    }

    /// Adds an object file, under a resource tracker.
    ///
    /// # Errors
    /// What stopped it being added.
    pub fn add_object_file_with_tracker(
        &self,
        tracker: &ResourceTracker<'_>,
        object: MemoryBuffer<'static>,
    ) -> Result<(), Error> {
        // SAFETY: both are live, and LLVM takes the buffer in every case.
        Error::from_raw_result(unsafe {
            raw::LLVMOrcLLJITAddObjectFileWithRT(self.raw, tracker.raw, object.into_raw())
        })
    }

    /// Adds an object file directly to the object layer, skipping any object transform.
    ///
    /// # Errors
    /// What stopped it being added.
    pub fn add_object_file_to_layer(&self, library: JitDylib<'_>, object: MemoryBuffer<'static>) -> Result<(), Error> {
        // SAFETY: both are live, and LLVM takes the buffer in every case.
        Error::from_raw_result(unsafe {
            raw::LLVMOrcObjectLayerAddObjectFile(
                raw::LLVMOrcLLJITGetObjLinkingLayer(self.raw),
                library.raw,
                object.into_raw(),
            )
        })
    }

    /// Adds an object file directly to the object layer, under a resource tracker.
    ///
    /// # Errors
    /// What stopped it being added.
    pub fn add_object_file_to_layer_with_tracker(
        &self,
        tracker: &ResourceTracker<'_>,
        object: MemoryBuffer<'static>,
    ) -> Result<(), Error> {
        // SAFETY: both are live, and LLVM takes the buffer in every case.
        Error::from_raw_result(unsafe {
            raw::LLVMOrcObjectLayerAddObjectFileWithRT(
                raw::LLVMOrcLLJITGetObjLinkingLayer(self.raw),
                tracker.raw,
                object.into_raw(),
            )
        })
    }

    /// The address of a symbol, making its code if it hasn't been. To call it, cast it to a function, which is `unsafe`.
    ///
    /// # Errors
    /// If there is no such symbol, or its code couldn't be made.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn lookup(&self, name: &str) -> Result<u64, Error> {
        let name = c_string(name, "lookup");
        let mut address = 0;
        // SAFETY: the JIT is live, the name is a valid C string, and LLVM writes the address when there is no error.
        Error::from_raw_result(unsafe { raw::LLVMOrcLLJITLookup(self.raw, &raw mut address, name.as_ptr()) })?;
        Ok(address)
    }

    /// Turns on debugger support for what this JIT makes.
    ///
    /// # Errors
    /// If this platform can't.
    pub fn enable_debug_support(&self) -> Result<(), Error> {
        // SAFETY: the JIT is live.
        Error::from_raw_result(unsafe { raw::LLVMOrcLLJITEnableDebugSupport(self.raw) })
    }

    /// Tells a listener what this JIT makes.
    ///
    /// # Panics
    /// If this JIT doesn't use `RuntimeDyld`, from [`JitBuilder::use_runtime_dyld`], which is the only linker LLVM lets one
    /// listen to (D3).
    #[track_caller]
    pub fn register_event_listener(&self, listener: &JitEventListener) {
        if !self.runtime_dyld {
            bad_call(format_args!("register_event_listener: this JIT doesn't use RuntimeDyld"));
        }
        // SAFETY: the layer is `RuntimeDyld`'s, as just checked, and LLVM owns the listener, which lasts for good.
        unsafe {
            raw::LLVMOrcRTDyldObjectLinkingLayerRegisterJITEventListener(
                raw::LLVMOrcLLJITGetObjLinkingLayer(self.raw),
                listener.raw,
            );
        }
    }

    /// Sets what runs on each module just before its code is made, such as optimization passes.
    ///
    /// The transform lasts as long as this JIT. A panic in it aborts, since it is running inside LLVM (D3).
    pub fn set_module_transform<F>(&self, transform: F)
    where
        F: for<'context> FnMut(&mut Module<'context>) -> Result<(), Error> + Send + 'static,
    {
        unsafe extern "C" fn each_module<F>(state: *mut c_void, module: raw::LLVMModuleRef) -> raw::LLVMErrorRef
        where
            F: for<'context> FnMut(&mut Module<'context>) -> Result<(), Error>,
        {
            crate::abort_on_panic(|| {
                // SAFETY: the state is the closure below, for this call; ORC holds the module's lock and gives it for this
                // call, exclusively.
                let (transform, mut wrapped) = unsafe { (&mut *state.cast::<F>(), Module::from_raw_borrowed(module)) };
                match transform(&mut wrapped) {
                    Ok(()) => std::ptr::null_mut(),
                    Err(error) => error.into_raw(),
                }
            })
        }
        unsafe extern "C" fn call<F>(
            state: *mut c_void,
            module: *mut raw::LLVMOrcThreadSafeModuleRef,
            _responsibility: raw::LLVMOrcMaterializationResponsibilityRef,
        ) -> raw::LLVMErrorRef
        where
            F: for<'context> FnMut(&mut Module<'context>) -> Result<(), Error>,
        {
            // SAFETY: LLVM gives a pointer to a live thread-safe module, and the closure this was made with.
            unsafe { raw::LLVMOrcThreadSafeModuleWithModuleDo(*module, Some(each_module::<F>), state) }
        }
        // A JIT's transform lives as long as the JIT, and LLVM has no way to free one, so it stays for good.
        let state = Box::into_raw(Box::new(transform)).cast::<c_void>();
        // SAFETY: the layer is live, the function matches LLVM's signature, and `state` lasts for good.
        unsafe {
            raw::LLVMOrcIRTransformLayerSetTransform(
                raw::LLVMOrcLLJITGetIRTransformLayer(self.raw),
                Some(call::<F>),
                state,
            );
        }
    }

    /// Sets what runs on each object file just after it is made: it can give a replacement.
    ///
    /// The transform lasts as long as this JIT. A panic in it aborts, since it is running inside LLVM (D3).
    pub fn set_object_transform<F>(&self, transform: F)
    where
        F: FnMut(&[u8]) -> Result<Option<Vec<u8>>, Error> + Send + 'static,
    {
        unsafe extern "C" fn call<F>(state: *mut c_void, object: *mut raw::LLVMMemoryBufferRef) -> raw::LLVMErrorRef
        where
            F: FnMut(&[u8]) -> Result<Option<Vec<u8>>, Error>,
        {
            crate::abort_on_panic(|| {
                // SAFETY: the state is the closure below; LLVM gives a live buffer it can have replaced.
                let (transform, current) =
                    unsafe { (&mut *state.cast::<F>(), std::mem::ManuallyDrop::new(MemoryBuffer::from_raw(*object))) };
                match transform(current.as_bytes()) {
                    Ok(None) => std::ptr::null_mut(),
                    Ok(Some(bytes)) => {
                        // SAFETY: the old buffer goes, and LLVM owns the new one from here on.
                        unsafe {
                            raw::LLVMDisposeMemoryBuffer(*object);
                            *object = MemoryBuffer::from_bytes(&bytes, "object").into_raw();
                        }
                        std::ptr::null_mut()
                    }
                    Err(error) => error.into_raw(),
                }
            })
        }
        // A JIT's transform lives as long as the JIT, and LLVM has no way to free one, so it stays for good.
        let state = Box::into_raw(Box::new(transform)).cast::<c_void>();
        // SAFETY: the layer is live, the function matches LLVM's signature, and `state` lasts for good.
        unsafe {
            raw::LLVMOrcObjectTransformLayerSetTransform(
                raw::LLVMOrcLLJITGetObjTransformLayer(self.raw),
                Some(call::<F>),
                state,
            );
        }
    }

    /// Sets what happens to an error ORC can't hand back to anything. A panic in it aborts, since it is running inside LLVM
    /// (D3).
    pub fn set_error_reporter<F: FnMut(Error) + Send + 'static>(&self, reporter: F) {
        unsafe extern "C" fn call<F: FnMut(Error)>(state: *mut c_void, error: raw::LLVMErrorRef) {
            crate::abort_on_panic(|| {
                // SAFETY: the state is the closure below.
                let reporter = unsafe { &mut *state.cast::<F>() };
                if let Err(found) = Error::from_raw_result(error) {
                    reporter(found);
                }
            });
        }
        // A reporter lives as long as the JIT, and LLVM has no way to free one, so it stays for good.
        let state = Box::into_raw(Box::new(reporter)).cast::<c_void>();
        // SAFETY: the session is live, the function matches LLVM's signature, and `state` lasts for good.
        unsafe { raw::LLVMOrcExecutionSessionSetErrorReporter(self.session(), Some(call::<F>), state) };
    }
}

impl<'jit> JitDylib<'jit> {
    /// A new resource tracker for this library.
    #[must_use]
    pub fn create_resource_tracker(self) -> ResourceTracker<'jit> {
        // SAFETY: the library is live for `'jit`; the wrapper holds the reference LLVM gives back.
        ResourceTracker { raw: unsafe { raw::LLVMOrcJITDylibCreateResourceTracker(self.raw) }, marker: PhantomData }
    }

    /// The tracker things added to this library go under by default.
    #[must_use]
    pub fn default_resource_tracker(self) -> ResourceTracker<'jit> {
        // SAFETY: the library is live for `'jit`; LLVM's call adds a reference for the wrapper to release.
        ResourceTracker { raw: unsafe { raw::LLVMOrcJITDylibGetDefaultResourceTracker(self.raw) }, marker: PhantomData }
    }

    /// Defines symbols at addresses that are already there, such as functions in the compiler itself.
    ///
    /// # Errors
    /// What stopped them being defined, such as a symbol already defined.
    pub fn define_absolute_symbols(
        self,
        symbols: &[(&SymbolStringPoolEntry, u64, JitSymbolFlags)],
    ) -> Result<(), Error> {
        let mut pairs: Vec<raw::LLVMOrcCSymbolMapPair> = symbols
            .iter()
            .map(|(name, address, flags)| raw::LLVMOrcCSymbolMapPair {
                // ORC takes a reference to each name from here on.
                Name: {
                    // SAFETY: the name is live.
                    unsafe { raw::LLVMOrcRetainSymbolStringPoolEntry(name.raw) };
                    name.raw
                },
                Sym: raw::LLVMJITEvaluatedSymbol { Address: *address, Flags: flags.to_raw() },
            })
            .collect();
        // SAFETY: LLVM reads the pairs and takes what they refer to; the unit it makes goes to the library in either case.
        Error::from_raw_result(unsafe {
            let unit = raw::LLVMOrcAbsoluteSymbols(pairs.as_mut_ptr(), pairs.len());
            raw::LLVMOrcJITDylibDefine(self.raw, unit)
        })
    }

    /// Removes everything from this library.
    ///
    /// # Errors
    /// What stopped it being cleared.
    pub fn clear(self) -> Result<(), Error> {
        // SAFETY: the library is live for `'jit`.
        Error::from_raw_result(unsafe { raw::LLVMOrcJITDylibClear(self.raw) })
    }

    /// Finds symbols in this process when nothing in the library defines them, such as libc's.
    ///
    /// # Errors
    /// What stopped LLVM reading this process's symbols.
    pub fn add_process_symbols(self, global_prefix: u8) -> Result<(), Error> {
        let mut generator = std::ptr::null_mut();
        // SAFETY: LLVM writes the generator when there is no error; there is no filter.
        Error::from_raw_result(unsafe {
            raw::LLVMOrcCreateDynamicLibrarySearchGeneratorForProcess(
                &raw mut generator,
                global_prefix.cast_signed(),
                None,
                std::ptr::null_mut(),
            )
        })?;
        // SAFETY: the library takes the generator.
        unsafe { raw::LLVMOrcJITDylibAddGenerator(self.raw, generator) };
        Ok(())
    }

    /// Finds symbols in a dynamic library on disk when nothing in this library defines them.
    ///
    /// # Errors
    /// What stopped LLVM reading it.
    pub fn add_dynamic_library_symbols(self, path: &std::path::Path, global_prefix: u8) -> Result<(), Error> {
        let path = crate::text::c_path(path)?;
        let mut generator = std::ptr::null_mut();
        // SAFETY: LLVM writes the generator when there is no error; there is no filter.
        Error::from_raw_result(unsafe {
            raw::LLVMOrcCreateDynamicLibrarySearchGeneratorForPath(
                &raw mut generator,
                path.as_ptr(),
                global_prefix.cast_signed(),
                None,
                std::ptr::null_mut(),
            )
        })?;
        // SAFETY: the library takes the generator.
        unsafe { raw::LLVMOrcJITDylibAddGenerator(self.raw, generator) };
        Ok(())
    }

    /// Finds symbols in a static library on disk when nothing in this library defines them.
    ///
    /// # Errors
    /// What stopped LLVM reading it.
    pub fn add_static_library_symbols(self, jit: &'jit Jit, path: &std::path::Path) -> Result<(), Error> {
        let path = crate::text::c_path(path)?;
        let mut generator = std::ptr::null_mut();
        // SAFETY: the object layer is live for `'jit`; LLVM writes the generator when there is no error.
        Error::from_raw_result(unsafe {
            raw::LLVMOrcCreateStaticLibrarySearchGeneratorForPath(
                &raw mut generator,
                raw::LLVMOrcLLJITGetObjLinkingLayer(jit.raw),
                path.as_ptr(),
            )
        })?;
        // SAFETY: the library takes the generator.
        unsafe { raw::LLVMOrcJITDylibAddGenerator(self.raw, generator) };
        Ok(())
    }
}

/// A transform that writes each object file to a directory, for looking at later.
pub struct DumpObjects {
    raw: raw::LLVMOrcDumpObjectsRef,
}

impl Drop for DumpObjects {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns it.
        unsafe { raw::LLVMOrcDisposeDumpObjects(self.raw) };
    }
}

impl DumpObjects {
    /// Writes to a directory, with names from `identifier_override` if not empty, and otherwise from each object.
    ///
    /// # Panics
    /// If a string has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn new(directory: &str, identifier_override: &str) -> Self {
        let directory = c_string(directory, "DumpObjects::new");
        let identifier_override = c_string(identifier_override, "DumpObjects::new");
        // SAFETY: LLVM copies the valid C strings, and the wrapper owns what it makes.
        DumpObjects { raw: unsafe { raw::LLVMOrcCreateDumpObjects(directory.as_ptr(), identifier_override.as_ptr()) } }
    }

    /// Writes one object.
    ///
    /// # Errors
    /// What stopped it being written.
    pub fn dump(&mut self, object: MemoryBuffer<'static>) -> Result<MemoryBuffer<'static>, Error> {
        let mut raw_buffer = object.into_raw();
        // SAFETY: LLVM gets a live buffer through the pointer, and leaves one there on success; on failure the buffer stays
        // there too, freed below.
        let result =
            Error::from_raw_result(unsafe { raw::LLVMOrcDumpObjects_CallOperator(self.raw, &raw mut raw_buffer) });
        // SAFETY: there is a live buffer at the pointer, as just said, and the wrapper owns it.
        let buffer = unsafe { MemoryBuffer::from_raw(raw_buffer) };
        result.map(|()| buffer)
    }
}
