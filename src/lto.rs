//! LLVM's link-time optimization library, `libLTO`.
//!
//! `libLTO` is a separate library with its own C API and its own state: it keeps one error message for the whole process, so
//! these errors should be read on the thread that hit them.

use std::ffi::{CString, c_char, c_int};
use std::marker::PhantomData;
use std::path::Path;

use crate::bad_call::bad_call;
use crate::error::Error;
use crate::raw;
use crate::text::{c_path, c_string, from_borrowed};

fn last_error() -> Error {
    // SAFETY: takes nothing; the text is copied right away.
    Error::new(from_borrowed(unsafe { raw::lto_get_error_message() }))
}

/// `libLTO`'s version text.
#[must_use]
pub fn version() -> String {
    // SAFETY: takes nothing; the text is copied right away.
    from_borrowed(unsafe { raw::lto_get_version() })
}

/// `libLTO`'s interface version.
#[must_use]
pub fn api_version() -> u32 {
    // SAFETY: takes nothing.
    unsafe { raw::lto_api_version() }
}

/// Sets up the disassembler for `libLTO`'s use.
pub fn initialize_disassembler() {
    // SAFETY: takes nothing.
    unsafe { raw::lto_initialize_disassembler() };
}

/// The names of the symbols the runtime library must provide.
#[must_use]
pub fn runtime_library_symbols() -> Vec<String> {
    let mut count = 0;
    // SAFETY: LLVM writes the count and keeps the list for good; each name is copied right away.
    unsafe {
        let list = raw::lto_runtime_lib_symbols_list(&raw mut count);
        (0..count).map(|index| from_borrowed(*list.add(index))).collect()
    }
}

/// Gives LLVM's code generator options, as `llc` takes them, for `libLTO`.
///
/// # Safety
/// LLVM's options are plain global data, so no other thread may be using LLVM during this call.
///
/// # Panics
/// If an option has a NUL byte inside it (D3).
#[track_caller]
pub unsafe fn set_debug_options(options: &[&str]) {
    with_c_list("set_debug_options", options, |list, count| {
        // SAFETY: the list lasts for the call, and LLVM copies what it keeps; the caller promises no other thread is in LLVM.
        unsafe { raw::lto_set_debug_options(list, count) };
    });
}

#[track_caller]
fn with_c_list<R>(operation: &str, items: &[&str], body: impl FnOnce(*const *const c_char, c_int) -> R) -> R {
    let owned: Vec<CString> = items.iter().map(|item| c_string(item, operation)).collect();
    let pointers: Vec<*const c_char> = owned.iter().map(|item| item.as_ptr()).collect();
    let Ok(count) = c_int::try_from(pointers.len()) else { bad_call(format_args!("{operation}: too many items")) };
    body(pointers.as_ptr(), count)
}

/// Whether a file is an object file `libLTO` can read.
///
/// # Errors
/// If the path has a NUL byte in it.
pub fn is_object_file(path: &Path) -> Result<bool, Error> {
    let path = c_path(path)?;
    // SAFETY: a valid C string for the call.
    Ok(unsafe { raw::lto_module_is_object_file(path.as_ptr()) })
}

/// Whether a file is an object file `libLTO` can read, for a target whose triple starts with this.
///
/// # Errors
/// If the path has a NUL byte in it.
///
/// # Panics
/// If the triple prefix has a NUL byte inside it (D3).
#[track_caller]
pub fn is_object_file_for_target(path: &Path, triple_prefix: &str) -> Result<bool, Error> {
    let path = c_path(path)?;
    let triple_prefix = c_string(triple_prefix, "is_object_file_for_target");
    // SAFETY: valid C strings for the call.
    Ok(unsafe { raw::lto_module_is_object_file_for_target(path.as_ptr(), triple_prefix.as_ptr()) })
}

/// Whether these bytes are an object file `libLTO` can read.
#[must_use]
pub fn is_object_file_in_memory(bytes: &[u8]) -> bool {
    // SAFETY: LLVM reads the length given.
    unsafe { raw::lto_module_is_object_file_in_memory(bytes.as_ptr().cast(), bytes.len()) }
}

/// Whether these bytes are an object file `libLTO` can read, for a target whose triple starts with this.
///
/// # Panics
/// If the triple prefix has a NUL byte inside it (D3).
#[must_use]
#[track_caller]
pub fn is_object_file_in_memory_for_target(bytes: &[u8], triple_prefix: &str) -> bool {
    let triple_prefix = c_string(triple_prefix, "is_object_file_in_memory_for_target");
    // SAFETY: LLVM reads the length given, and the prefix is a valid C string.
    unsafe {
        raw::lto_module_is_object_file_in_memory_for_target(bytes.as_ptr().cast(), bytes.len(), triple_prefix.as_ptr())
    }
}

/// Whether these bytes hold an Objective-C category.
#[must_use]
pub fn has_objective_c_category(bytes: &[u8]) -> bool {
    // SAFETY: LLVM reads the length given.
    unsafe { raw::lto_module_has_objc_category(bytes.as_ptr().cast(), bytes.len()) }
}

/// An object file read by `libLTO`.
pub struct LtoModule {
    raw: raw::lto_module_t,
    // LLVM's `LTOModule` keeps a reference to the bytes it read, so they live here, as long as it does.
    bytes: Option<Box<[u8]>>,
    context: LtoContext,
}

/// Which context a module was read into. `libLTO` can't say, and it can only merge modules within one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LtoContext {
    Global,
    OwnContext,
    Generator(raw::lto_code_gen_t),
}

impl Drop for LtoModule {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the module.
        unsafe { raw::lto_module_dispose(self.raw) };
    }
}

/// What `libLTO` knows about a symbol, as LLVM packs it into one number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LtoSymbolAttributes(u32);

impl LtoSymbolAttributes {
    /// The symbol's alignment, as a power of two.
    #[must_use]
    pub fn alignment_as_power_of_two(self) -> u32 {
        self.0 & raw::lto_symbol_attributes::LTO_SYMBOL_ALIGNMENT_MASK.0
    }

    /// Whether the symbol is code.
    #[must_use]
    pub fn is_code(self) -> bool {
        self.0 & raw::lto_symbol_attributes::LTO_SYMBOL_PERMISSIONS_MASK.0
            == raw::lto_symbol_attributes::LTO_SYMBOL_PERMISSIONS_CODE.0
    }

    /// Whether the symbol is undefined here.
    #[must_use]
    pub fn is_undefined(self) -> bool {
        self.0 & raw::lto_symbol_attributes::LTO_SYMBOL_DEFINITION_MASK.0
            == raw::lto_symbol_attributes::LTO_SYMBOL_DEFINITION_UNDEFINED.0
    }

    /// Whether the symbol is weak.
    #[must_use]
    pub fn is_weak(self) -> bool {
        self.0 & raw::lto_symbol_attributes::LTO_SYMBOL_DEFINITION_MASK.0
            == raw::lto_symbol_attributes::LTO_SYMBOL_DEFINITION_WEAK.0
    }

    /// Whether the symbol is only seen inside its file.
    #[must_use]
    pub fn is_internal(self) -> bool {
        self.0 & raw::lto_symbol_attributes::LTO_SYMBOL_SCOPE_MASK.0
            == raw::lto_symbol_attributes::LTO_SYMBOL_SCOPE_INTERNAL.0
    }

    /// Whether the symbol is visible everywhere.
    #[must_use]
    pub fn is_default_scope(self) -> bool {
        self.0 & raw::lto_symbol_attributes::LTO_SYMBOL_SCOPE_MASK.0
            == raw::lto_symbol_attributes::LTO_SYMBOL_SCOPE_DEFAULT.0
    }

    /// Whether the symbol is in a COMDAT.
    #[must_use]
    pub fn is_comdat(self) -> bool {
        self.0 & raw::lto_symbol_attributes::LTO_SYMBOL_COMDAT.0 != 0
    }

    /// Whether the symbol is an alias.
    #[must_use]
    pub fn is_alias(self) -> bool {
        self.0 & raw::lto_symbol_attributes::LTO_SYMBOL_ALIAS.0 != 0
    }
}

impl LtoModule {
    fn from_raw_or_error(
        raw_module: raw::lto_module_t,
        bytes: Option<Box<[u8]>>,
        context: LtoContext,
    ) -> Result<Self, Error> {
        if raw_module.is_null() { Err(last_error()) } else { Ok(LtoModule { raw: raw_module, bytes, context }) }
    }

    /// Reads a file.
    ///
    /// # Errors
    /// What is wrong with it.
    pub fn from_file(path: &Path) -> Result<Self, Error> {
        let path = c_path(path)?;
        // SAFETY: a valid C string for the call; the wrapper owns what comes back.
        Self::from_raw_or_error(unsafe { raw::lto_module_create(path.as_ptr()) }, None, LtoContext::Global)
    }

    /// Reads bytes, which it copies.
    ///
    /// # Errors
    /// What is wrong with them.
    pub fn from_memory(bytes: &[u8]) -> Result<Self, Error> {
        let bytes = Box::<[u8]>::from(bytes);
        // SAFETY: LLVM reads the length given from the copy, which the wrapper keeps as long as what comes back.
        Self::from_raw_or_error(
            unsafe { raw::lto_module_create_from_memory(bytes.as_ptr().cast(), bytes.len()) },
            Some(bytes),
            LtoContext::Global,
        )
    }

    /// Reads bytes, which it copies, with a path to name it by.
    ///
    /// # Errors
    /// What is wrong with them.
    ///
    /// # Panics
    /// If the path has a NUL byte inside it (D3).
    #[track_caller]
    pub fn from_memory_with_path(bytes: &[u8], path: &str) -> Result<Self, Error> {
        let path = c_string(path, "from_memory_with_path");
        let bytes = Box::<[u8]>::from(bytes);
        // SAFETY: LLVM reads the length given from the copy, which the wrapper keeps as long as what comes back.
        Self::from_raw_or_error(
            unsafe { raw::lto_module_create_from_memory_with_path(bytes.as_ptr().cast(), bytes.len(), path.as_ptr()) },
            Some(bytes),
            LtoContext::Global,
        )
    }

    /// Reads bytes, which it copies, into a context of its own, so that many can be read at once on several threads.
    ///
    /// # Errors
    /// What is wrong with them.
    ///
    /// # Panics
    /// If the path has a NUL byte inside it (D3).
    #[track_caller]
    pub fn in_local_context(bytes: &[u8], path: &str) -> Result<Self, Error> {
        let path = c_string(path, "in_local_context");
        let bytes = Box::<[u8]>::from(bytes);
        // SAFETY: LLVM reads the length given from the copy, which the wrapper keeps as long as what comes back.
        Self::from_raw_or_error(
            unsafe { raw::lto_module_create_in_local_context(bytes.as_ptr().cast(), bytes.len(), path.as_ptr()) },
            Some(bytes),
            LtoContext::OwnContext,
        )
    }

    /// The target triple.
    #[must_use]
    pub fn target_triple(&self) -> String {
        // SAFETY: the module is live; the text is copied right away.
        from_borrowed(unsafe { raw::lto_module_get_target_triple(self.raw) })
    }

    /// Sets the target triple.
    ///
    /// # Panics
    /// If `triple` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn set_target_triple(&mut self, triple: &str) {
        let triple = c_string(triple, "set_target_triple");
        // SAFETY: the module is live, and LLVM copies the valid C string.
        unsafe { raw::lto_module_set_target_triple(self.raw, triple.as_ptr()) };
    }

    /// How many symbols it has.
    #[must_use]
    pub fn symbol_count(&self) -> u32 {
        // SAFETY: the module is live.
        unsafe { raw::lto_module_get_num_symbols(self.raw) }
    }

    /// A symbol's name.
    ///
    /// # Panics
    /// If there is no such symbol (D3).
    #[must_use]
    #[track_caller]
    pub fn symbol_name(&self, index: u32) -> String {
        let count = self.symbol_count();
        if !crate::checks::index_in_bounds(index, count) {
            bad_call(format_args!("symbol_name: index {index} is out of bounds for {count} symbols"));
        }
        // SAFETY: the index was just checked; the text is copied right away.
        from_borrowed(unsafe { raw::lto_module_get_symbol_name(self.raw, index) })
    }

    /// What it knows about a symbol.
    ///
    /// # Panics
    /// If there is no such symbol (D3).
    #[must_use]
    #[track_caller]
    pub fn symbol_attributes(&self, index: u32) -> LtoSymbolAttributes {
        let count = self.symbol_count();
        if !crate::checks::index_in_bounds(index, count) {
            bad_call(format_args!("symbol_attributes: index {index} is out of bounds for {count} symbols"));
        }
        // SAFETY: the index was just checked.
        LtoSymbolAttributes(unsafe { raw::lto_module_get_symbol_attribute(self.raw, index) }.0)
    }

    /// The undefined symbols its module-level assembly uses.
    #[must_use]
    pub fn assembly_undefined_symbols(&self) -> Vec<String> {
        // SAFETY: the module is live, each index is below the count it gave, and each name is copied right away.
        unsafe {
            (0..raw::lto_module_get_num_asm_undef_symbols(self.raw))
                .map(|index| from_borrowed(raw::lto_module_get_asm_undef_symbol_name(self.raw, index)))
                .collect()
        }
    }

    /// The linker options it asks for.
    #[must_use]
    pub fn linker_options(&self) -> String {
        // SAFETY: the module is live; the text is copied right away.
        from_borrowed(unsafe { raw::lto_module_get_linkeropts(self.raw) })
    }

    /// Its Mach-O CPU type and subtype.
    ///
    /// # Errors
    /// If it has none.
    pub fn macho_cpu_type(&self) -> Result<(u32, u32), Error> {
        let (mut cpu_type, mut cpu_subtype) = (0, 0);
        // SAFETY: the module is live, and LLVM writes two numbers.
        let failed = unsafe { raw::lto_module_get_macho_cputype(self.raw, &raw mut cpu_type, &raw mut cpu_subtype) };
        if failed { Err(last_error()) } else { Ok((cpu_type, cpu_subtype)) }
    }

    /// Whether it has static constructors or destructors.
    #[must_use]
    pub fn has_constructors_or_destructors(&self) -> bool {
        // SAFETY: the module is live.
        unsafe { raw::lto_module_has_ctor_dtor(self.raw) }
    }

    /// Whether this is bitcode for `ThinLTO`.
    #[must_use]
    pub fn is_thin(&self) -> bool {
        // SAFETY: the module is live.
        unsafe { raw::lto_module_is_thinlto(self.raw) }
    }
}

/// How `libLTO` should make position-dependent code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LtoPositionModel {
    /// Static.
    Static,
    /// Dynamic.
    Dynamic,
    /// Dynamic with no position-independent code.
    DynamicNoPositionIndependentCode,
    /// The default.
    Default,
}

impl LtoPositionModel {
    fn to_raw(self) -> raw::lto_codegen_model {
        use raw::lto_codegen_model as M;
        match self {
            LtoPositionModel::Static => M::LTO_CODEGEN_PIC_MODEL_STATIC,
            LtoPositionModel::Dynamic => M::LTO_CODEGEN_PIC_MODEL_DYNAMIC,
            LtoPositionModel::DynamicNoPositionIndependentCode => M::LTO_CODEGEN_PIC_MODEL_DYNAMIC_NO_PIC,
            LtoPositionModel::Default => M::LTO_CODEGEN_PIC_MODEL_DEFAULT,
        }
    }
}

/// Whether what `libLTO` makes carries debug information.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LtoDebugModel {
    /// None.
    None,
    /// DWARF.
    Dwarf,
}

/// Merges modules, optimizes them and makes one object. It holds the modules it is given, for `'modules`.
pub struct LtoCodeGenerator<'modules> {
    raw: raw::lto_code_gen_t,
    in_global_context: bool,
    // Modules read into this generator's own context, which have to go before it does.
    read_modules: Vec<LtoModule>,
    // The bytes of modules LLVM took over: it goes on using them until the generator goes.
    kept_bytes: Vec<Box<[u8]>>,
    marker: PhantomData<&'modules ()>,
}

impl Drop for LtoCodeGenerator<'_> {
    fn drop(&mut self) {
        self.read_modules.clear();
        // SAFETY: this wrapper owns the generator, and nothing is left in its context. The kept bytes go after this.
        unsafe { raw::lto_codegen_dispose(self.raw) };
    }
}

impl<'modules> LtoCodeGenerator<'modules> {
    /// A generator in `libLTO`'s global context, for modules from [`LtoModule::from_file`] and [`LtoModule::from_memory`].
    #[must_use]
    pub fn new() -> Self {
        // SAFETY: takes nothing; the wrapper owns what comes back.
        let raw = unsafe { raw::lto_codegen_create() };
        LtoCodeGenerator {
            raw,
            in_global_context: true,
            read_modules: Vec::new(),
            kept_bytes: Vec::new(),
            marker: PhantomData,
        }
    }

    /// A generator in a context of its own, whose modules come through [`LtoCodeGenerator::read_module`].
    #[must_use]
    pub fn in_local_context() -> Self {
        // SAFETY: takes nothing; the wrapper owns what comes back.
        let raw = unsafe { raw::lto_codegen_create_in_local_context() };
        LtoCodeGenerator {
            raw,
            in_global_context: false,
            read_modules: Vec::new(),
            kept_bytes: Vec::new(),
            marker: PhantomData,
        }
    }

    // trace: D3
    #[track_caller]
    fn check_context(&self, call: &str, module: &LtoModule) {
        let matches = if self.in_global_context {
            module.context == LtoContext::Global
        } else {
            module.context == LtoContext::Generator(self.raw)
        };
        if !matches {
            bad_call(format_args!("{call}: this module was read into a different context from this generator's"));
        }
    }

    /// Reads bytes, which it copies, into this generator's context, and adds the module to what is merged.
    ///
    /// # Errors
    /// What is wrong with the bytes, or what stopped the module being added.
    ///
    /// # Panics
    /// If the path has a NUL byte inside it (D3).
    #[track_caller]
    pub fn read_module(&mut self, bytes: &[u8], path: &str) -> Result<(), Error> {
        let path = c_string(path, "read_module");
        let bytes = Box::<[u8]>::from(bytes);
        let context = if self.in_global_context { LtoContext::Global } else { LtoContext::Generator(self.raw) };
        // SAFETY: LLVM reads the length given from the copy, which the wrapper keeps as long as what comes back, and this
        // generator keeps that wrapper until just before it goes.
        let module = LtoModule::from_raw_or_error(
            unsafe {
                raw::lto_module_create_in_codegen_context(bytes.as_ptr().cast(), bytes.len(), path.as_ptr(), self.raw)
            },
            Some(bytes),
            context,
        )?;
        // SAFETY: both are live, in one context, and the module stays so as long as the generator.
        let failed = unsafe { raw::lto_codegen_add_module(self.raw, module.raw) };
        self.read_modules.push(module);
        if failed { Err(last_error()) } else { Ok(()) }
    }

    /// Adds a module to what is merged.
    ///
    /// # Errors
    /// What stopped it being added.
    ///
    /// # Panics
    /// If the module was read into a different context from this generator's (D3).
    #[track_caller]
    pub fn add_module(&mut self, module: &'modules LtoModule) -> Result<(), Error> {
        self.check_context("add_module", module);
        // SAFETY: both are live, in one context, and the module stays so for `'modules`.
        if unsafe { raw::lto_codegen_add_module(self.raw, module.raw) } { Err(last_error()) } else { Ok(()) }
    }

    /// Makes this module the first of what is merged, in place of anything added before. The generator owns it from here on.
    ///
    /// # Panics
    /// If the module was read into a different context from this generator's (D3).
    #[track_caller]
    pub fn set_module(&mut self, mut module: LtoModule) {
        self.check_context("set_module", &module);
        let raw_module = module.raw;
        self.kept_bytes.extend(module.bytes.take());
        std::mem::forget(module);
        // SAFETY: both are live, in one context, the generator takes the module, and it keeps the bytes as long as itself.
        unsafe { raw::lto_codegen_set_module(self.raw, raw_module) };
    }

    /// Sets whether what it makes carries debug information.
    ///
    /// # Errors
    /// If it couldn't be set.
    pub fn set_debug_model(&mut self, model: LtoDebugModel) -> Result<(), Error> {
        let raw_model = match model {
            LtoDebugModel::None => raw::lto_debug_model::LTO_DEBUG_MODEL_NONE,
            LtoDebugModel::Dwarf => raw::lto_debug_model::LTO_DEBUG_MODEL_DWARF,
        };
        // SAFETY: the generator is live.
        if unsafe { raw::lto_codegen_set_debug_model(self.raw, raw_model) } { Err(last_error()) } else { Ok(()) }
    }

    /// Sets how it makes position-dependent code.
    ///
    /// # Errors
    /// If it couldn't be set.
    pub fn set_position_model(&mut self, model: LtoPositionModel) -> Result<(), Error> {
        // SAFETY: the generator is live.
        if unsafe { raw::lto_codegen_set_pic_model(self.raw, model.to_raw()) } { Err(last_error()) } else { Ok(()) }
    }

    /// Sets the processor.
    ///
    /// # Panics
    /// If `cpu` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn set_cpu(&mut self, cpu: &str) {
        let cpu = c_string(cpu, "set_cpu");
        // SAFETY: the generator is live, and LLVM copies the valid C string.
        unsafe { raw::lto_codegen_set_cpu(self.raw, cpu.as_ptr()) };
    }

    /// Sets the assembler to run, for targets that need one.
    ///
    /// # Errors
    /// If the path has a NUL byte in it.
    pub fn set_assembler_path(&mut self, path: &Path) -> Result<(), Error> {
        let path = c_path(path)?;
        // SAFETY: the generator is live, and LLVM copies the valid C string.
        unsafe { raw::lto_codegen_set_assembler_path(self.raw, path.as_ptr()) };
        Ok(())
    }

    /// Sets the arguments of the assembler.
    ///
    /// # Panics
    /// If an argument has a NUL byte inside it (D3).
    #[track_caller]
    pub fn set_assembler_arguments(&mut self, arguments: &[&str]) {
        with_c_list("set_assembler_arguments", arguments, |list, count| {
            // SAFETY: the generator is live, and LLVM copies what it keeps. Its signature wants a mutable pointer by old
            // habit.
            unsafe { raw::lto_codegen_set_assembler_args(self.raw, list.cast_mut(), count) };
        });
    }

    /// Keeps a symbol through the dead-code passes.
    ///
    /// # Panics
    /// If `symbol` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn add_must_preserve_symbol(&mut self, symbol: &str) {
        let symbol = c_string(symbol, "add_must_preserve_symbol");
        // SAFETY: the generator is live, and LLVM copies the valid C string.
        unsafe { raw::lto_codegen_add_must_preserve_symbol(self.raw, symbol.as_ptr()) };
    }

    /// Writes the merged modules as bitcode to a file.
    ///
    /// # Errors
    /// What stopped them being written.
    pub fn write_merged_modules(&mut self, path: &Path) -> Result<(), Error> {
        let path = c_path(path)?;
        // SAFETY: the generator is live, and the path is a valid C string for the call.
        if unsafe { raw::lto_codegen_write_merged_modules(self.raw, path.as_ptr()) } {
            Err(last_error())
        } else {
            Ok(())
        }
    }

    fn copy_output(pointer: *const std::ffi::c_void, size_in_bytes: usize) -> Result<Vec<u8>, Error> {
        if pointer.is_null() {
            return Err(last_error());
        }
        // SAFETY: LLVM says where the object is and how big; it is copied right away, before anything can replace it.
        Ok(unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size_in_bytes) }.to_vec())
    }

    /// Optimizes and makes one object.
    ///
    /// # Errors
    /// What stopped it being made.
    pub fn compile(&mut self) -> Result<Vec<u8>, Error> {
        let mut size_in_bytes = 0;
        // SAFETY: the generator is live, and LLVM writes the size beside the pointer it gives back.
        let output = unsafe { raw::lto_codegen_compile(self.raw, &raw mut size_in_bytes) };
        Self::copy_output(output, size_in_bytes)
    }

    /// Optimizes and makes one object in a file of its choosing, and says which.
    ///
    /// # Errors
    /// What stopped it being made.
    pub fn compile_to_file(&mut self) -> Result<String, Error> {
        let mut name = std::ptr::null();
        // SAFETY: the generator is live, and LLVM writes the name, copied right away.
        if unsafe { raw::lto_codegen_compile_to_file(self.raw, &raw mut name) } {
            return Err(last_error());
        }
        Ok(from_borrowed(name))
    }

    /// Optimizes, without making code.
    ///
    /// # Errors
    /// What stopped it.
    pub fn optimize(&mut self) -> Result<(), Error> {
        // SAFETY: the generator is live.
        if unsafe { raw::lto_codegen_optimize(self.raw) } { Err(last_error()) } else { Ok(()) }
    }

    /// Makes one object from what [`LtoCodeGenerator::optimize`] did.
    ///
    /// # Errors
    /// What stopped it being made.
    pub fn compile_optimized(&mut self) -> Result<Vec<u8>, Error> {
        let mut size_in_bytes = 0;
        // SAFETY: the generator is live, and LLVM writes the size beside the pointer it gives back.
        let output = unsafe { raw::lto_codegen_compile_optimized(self.raw, &raw mut size_in_bytes) };
        Self::copy_output(output, size_in_bytes)
    }

    /// Gives its code generator options, as `llc` takes them, separated by spaces.
    ///
    /// # Safety
    /// LLVM's options are plain global data, so no other thread may be using LLVM during the next compile.
    ///
    /// # Panics
    /// If `options` has a NUL byte inside it (D3).
    #[track_caller]
    pub unsafe fn debug_options(&mut self, options: &str) {
        let options = c_string(options, "debug_options");
        // SAFETY: the generator is live, and LLVM copies the valid C string; the caller keeps other threads out of LLVM.
        unsafe { raw::lto_codegen_debug_options(self.raw, options.as_ptr()) };
    }

    /// Gives its code generator options, as `llc` takes them, one per item.
    ///
    /// # Safety
    /// LLVM's options are plain global data, so no other thread may be using LLVM during the next compile.
    ///
    /// # Panics
    /// If an option has a NUL byte inside it (D3).
    #[track_caller]
    pub unsafe fn debug_options_list(&mut self, options: &[&str]) {
        with_c_list("debug_options_list", options, |list, count| {
            // SAFETY: the generator is live, and LLVM copies what it keeps; the caller keeps other threads out of LLVM.
            unsafe { raw::lto_codegen_debug_options_array(self.raw, list, count) };
        });
    }

    /// Sets whether it should make symbols it doesn't need to keep internal.
    pub fn set_should_internalize(&mut self, should: bool) {
        // SAFETY: the generator is live.
        unsafe { raw::lto_codegen_set_should_internalize(self.raw, should) };
    }

    /// Sets whether it should keep use lists in bitcode.
    pub fn set_should_embed_use_lists(&mut self, should: bool) {
        // SAFETY: the generator is live.
        unsafe { raw::lto_codegen_set_should_embed_uselists(self.raw, should) };
    }

    /// Sets what happens to a diagnostic `libLTO` reports. A panic in it aborts, since it is running inside LLVM (D3).
    ///
    /// The handler lives until the generator goes, so setting one again leaks the old one.
    pub fn set_diagnostic_handler<F: FnMut(LtoDiagnosticSeverity, String) + 'modules>(&mut self, handler: F) {
        unsafe extern "C" fn call<F: FnMut(LtoDiagnosticSeverity, String)>(
            severity: raw::lto_codegen_diagnostic_severity_t,
            message: *const c_char,
            state: *mut std::ffi::c_void,
        ) {
            crate::abort_on_panic(|| {
                // SAFETY: LLVM passes back the state given with this function, which is the handler, and a message for this
                // call, copied here.
                use raw::lto_codegen_diagnostic_severity_t as S;
                let (handler, message) = unsafe { (&mut *state.cast::<F>(), from_borrowed(message)) };
                let severity = match severity {
                    S::LTO_DS_ERROR => LtoDiagnosticSeverity::Error,
                    S::LTO_DS_WARNING => LtoDiagnosticSeverity::Warning,
                    S::LTO_DS_REMARK => LtoDiagnosticSeverity::Remark,
                    _ => LtoDiagnosticSeverity::Note,
                };
                handler(severity, message);
            });
        }
        let state = Box::into_raw(Box::new(handler)).cast::<std::ffi::c_void>();
        // SAFETY: the generator is live, the function matches LLVM's signature, and `state` is leaked, so it lasts.
        unsafe { raw::lto_codegen_set_diagnostic_handler(self.raw, Some(call::<F>), state) };
    }
}

impl Default for LtoCodeGenerator<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// How serious a diagnostic from `libLTO` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LtoDiagnosticSeverity {
    /// An error.
    Error,
    /// A warning.
    Warning,
    /// A remark.
    Remark,
    /// A note.
    Note,
}

/// An input to a link, for asking about before reading it.
pub struct LtoInput<'data> {
    raw: raw::lto_input_t,
    marker: PhantomData<&'data [u8]>,
}

impl Drop for LtoInput<'_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the input.
        unsafe { raw::lto_input_dispose(self.raw) };
    }
}

impl<'data> LtoInput<'data> {
    /// An input from bytes, which it borrows, with a path to name it by.
    ///
    /// # Errors
    /// What is wrong with it.
    ///
    /// # Panics
    /// If the path has a NUL byte inside it (D3).
    #[track_caller]
    pub fn new(bytes: &'data [u8], path: &str) -> Result<Self, Error> {
        let path = c_string(path, "LtoInput::new");
        // SAFETY: the bytes last for `'data`, and the path is a valid C string for the call.
        let input = unsafe { raw::lto_input_create(bytes.as_ptr().cast(), bytes.len(), path.as_ptr()) };
        if input.is_null() { Err(last_error()) } else { Ok(LtoInput { raw: input, marker: PhantomData }) }
    }

    /// The libraries it says it depends on.
    #[must_use]
    pub fn dependent_libraries(&self) -> Vec<String> {
        // SAFETY: the input is live, each index is below the count it gave, and each name is copied right away.
        unsafe {
            (0..raw::lto_input_get_num_dependent_libraries(self.raw))
                .map(|index| {
                    let mut length = 0;
                    let name = raw::lto_input_get_dependent_library(self.raw, index as usize, &raw mut length);
                    crate::text::from_pointer_and_length(name, length)
                })
                .collect()
        }
    }
}

/// Runs `ThinLTO` over modules.
pub struct ThinLtoCodeGenerator {
    raw: raw::thinlto_code_gen_t,
}

impl Drop for ThinLtoCodeGenerator {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the generator.
        unsafe { raw::thinlto_codegen_dispose(self.raw) };
    }
}

impl Default for ThinLtoCodeGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl ThinLtoCodeGenerator {
    /// A new generator.
    #[must_use]
    pub fn new() -> Self {
        // SAFETY: takes nothing; the wrapper owns what comes back.
        ThinLtoCodeGenerator { raw: unsafe { raw::thinlto_create_codegen() } }
    }

    /// Adds a module from its bitcode, which must live until [`ThinLtoCodeGenerator::process`] is done, so it is held here by
    /// the `'static` bound.
    ///
    /// # Panics
    /// If the identifier has a NUL byte inside it, or the data is too long (D3).
    #[track_caller]
    pub fn add_module(&mut self, identifier: &str, bitcode: &'static [u8]) {
        let identifier = c_string(identifier, "add_module");
        let Ok(length) = c_int::try_from(bitcode.len()) else {
            bad_call(format_args!("add_module: {} bytes is more than libLTO can take", bitcode.len()))
        };
        // SAFETY: the generator is live, LLVM copies the valid C string, and the bytes last for good.
        unsafe { raw::thinlto_codegen_add_module(self.raw, identifier.as_ptr(), bitcode.as_ptr().cast(), length) };
    }

    /// Optimizes the modules and makes their objects.
    pub fn process(&mut self) {
        // SAFETY: the generator is live.
        unsafe { raw::thinlto_codegen_process(self.raw) };
    }

    /// The objects made in memory, one per module.
    #[must_use]
    pub fn objects(&self) -> Vec<Vec<u8>> {
        // SAFETY: the generator is live, each index is below the count it gave, and each buffer is copied right away.
        unsafe {
            (0..raw::thinlto_module_get_num_objects(self.raw))
                .map(|index| {
                    let object = raw::thinlto_module_get_object(self.raw, index);
                    std::slice::from_raw_parts(object.Buffer.cast::<u8>(), object.Size).to_vec()
                })
                .collect()
        }
    }

    /// The object files made on disk, one per module, when a directory was set for them.
    #[must_use]
    pub fn object_files(&self) -> Vec<String> {
        // SAFETY: the generator is live, each index is below the count it gave, and each name is copied right away.
        unsafe {
            (0..raw::thinlto_module_get_num_object_files(self.raw))
                .map(|index| from_borrowed(raw::thinlto_module_get_object_file(self.raw, index)))
                .collect()
        }
    }

    /// Sets how it makes position-dependent code.
    ///
    /// # Errors
    /// If it couldn't be set.
    pub fn set_position_model(&mut self, model: LtoPositionModel) -> Result<(), Error> {
        // SAFETY: the generator is live.
        if unsafe { raw::thinlto_codegen_set_pic_model(self.raw, model.to_raw()) } { Err(last_error()) } else { Ok(()) }
    }

    /// Sets where it keeps its intermediate files.
    ///
    /// # Errors
    /// If the path has a NUL byte in it.
    pub fn set_save_temporaries_directory(&mut self, path: &Path) -> Result<(), Error> {
        let path = c_path(path)?;
        // SAFETY: the generator is live, and LLVM copies the valid C string.
        unsafe { raw::thinlto_codegen_set_savetemps_dir(self.raw, path.as_ptr()) };
        Ok(())
    }

    /// Sets where it writes object files, instead of keeping them in memory.
    ///
    /// # Errors
    /// If the path has a NUL byte in it.
    pub fn set_generated_objects_directory(&mut self, path: &Path) -> Result<(), Error> {
        let path = c_path(path)?;
        // SAFETY: the generator is live, and LLVM copies the valid C string.
        unsafe { raw::thinlto_set_generated_objects_dir(self.raw, path.as_ptr()) };
        Ok(())
    }

    /// Sets the processor.
    ///
    /// # Panics
    /// If `cpu` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn set_cpu(&mut self, cpu: &str) {
        let cpu = c_string(cpu, "set_cpu");
        // SAFETY: the generator is live, and LLVM copies the valid C string.
        unsafe { raw::thinlto_codegen_set_cpu(self.raw, cpu.as_ptr()) };
    }

    /// Sets whether it stops before making code.
    pub fn disable_code_generation(&mut self, disable: bool) {
        // SAFETY: the generator is live.
        unsafe { raw::thinlto_codegen_disable_codegen(self.raw, disable) };
    }

    /// Sets whether it only makes code, from modules already optimized.
    pub fn set_code_generation_only(&mut self, code_generation_only: bool) {
        // SAFETY: the generator is live.
        unsafe { raw::thinlto_codegen_set_codegen_only(self.raw, code_generation_only) };
    }

    #[track_caller]
    fn name_length(operation: &str, name: &str) -> c_int {
        match c_int::try_from(name.len()) {
            Ok(length) => length,
            Err(_) => bad_call(format_args!("{operation}: {} bytes is more than libLTO can take", name.len())),
        }
    }

    /// Keeps a symbol through the dead-code passes.
    ///
    /// # Panics
    /// If the name is too long (D3).
    #[track_caller]
    pub fn add_must_preserve_symbol(&mut self, name: &str) {
        let length = Self::name_length("add_must_preserve_symbol", name);
        // SAFETY: the generator is live, and LLVM reads the length given.
        unsafe { raw::thinlto_codegen_add_must_preserve_symbol(self.raw, name.as_ptr().cast(), length) };
    }

    /// Says a symbol is used from native code that crosses into these modules.
    ///
    /// # Panics
    /// If the name is too long (D3).
    #[track_caller]
    pub fn add_cross_referenced_symbol(&mut self, name: &str) {
        let length = Self::name_length("add_cross_referenced_symbol", name);
        // SAFETY: the generator is live, and LLVM reads the length given.
        unsafe { raw::thinlto_codegen_add_cross_referenced_symbol(self.raw, name.as_ptr().cast(), length) };
    }

    /// Sets the directory of its cache.
    ///
    /// # Errors
    /// If the path has a NUL byte in it.
    pub fn set_cache_directory(&mut self, path: &Path) -> Result<(), Error> {
        let path = c_path(path)?;
        // SAFETY: the generator is live, and LLVM copies the valid C string.
        unsafe { raw::thinlto_codegen_set_cache_dir(self.raw, path.as_ptr()) };
        Ok(())
    }

    /// Sets how often it prunes its cache.
    ///
    /// # Panics
    /// If it is too long for `libLTO` (D3).
    #[track_caller]
    pub fn set_cache_pruning_interval_in_seconds(&mut self, interval_in_seconds: u32) {
        let Ok(interval) = c_int::try_from(interval_in_seconds) else {
            bad_call(format_args!("set_cache_pruning_interval_in_seconds: {interval_in_seconds} is too long"))
        };
        // SAFETY: the generator is live.
        unsafe { raw::thinlto_codegen_set_cache_pruning_interval(self.raw, interval) };
    }

    /// Sets the largest its cache can be, as a percentage of the available space.
    pub fn set_final_cache_size_relative_to_available_space(&mut self, percentage: u32) {
        // SAFETY: the generator is live.
        unsafe { raw::thinlto_codegen_set_final_cache_size_relative_to_available_space(self.raw, percentage) };
    }

    /// Sets how long a cache entry stays.
    pub fn set_cache_entry_expiration_in_seconds(&mut self, expiration_in_seconds: u32) {
        // SAFETY: the generator is live.
        unsafe { raw::thinlto_codegen_set_cache_entry_expiration(self.raw, expiration_in_seconds) };
    }

    /// Sets the largest its cache can be.
    pub fn set_cache_size_in_bytes(&mut self, size_in_bytes: u32) {
        // SAFETY: the generator is live.
        unsafe { raw::thinlto_codegen_set_cache_size_bytes(self.raw, size_in_bytes) };
    }

    /// Sets the largest its cache can be.
    pub fn set_cache_size_in_megabytes(&mut self, size_in_megabytes: u32) {
        // SAFETY: the generator is live.
        unsafe { raw::thinlto_codegen_set_cache_size_megabytes(self.raw, size_in_megabytes) };
    }

    /// Sets how many files its cache can hold.
    pub fn set_cache_size_in_files(&mut self, size_in_files: u32) {
        // SAFETY: the generator is live.
        unsafe { raw::thinlto_codegen_set_cache_size_files(self.raw, size_in_files) };
    }
}

/// Gives LLVM's code generator options, as `llc` takes them, for `ThinLTO`.
///
/// # Safety
/// LLVM's options are plain global data, so no other thread may be using LLVM during this call.
///
/// # Panics
/// If an option has a NUL byte inside it (D3).
#[track_caller]
pub unsafe fn set_thin_debug_options(options: &[&str]) {
    with_c_list("set_thin_debug_options", options, |list, count| {
        // SAFETY: the list lasts for the call, and LLVM copies what it keeps; the caller promises no other thread is in LLVM.
        unsafe { raw::thinlto_debug_options(list, count) };
    });
}
