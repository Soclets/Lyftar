//! Modules, and deleting from them (D11).

use std::marker::PhantomData;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::bad_call::bad_call;
use crate::block::Block;
use crate::checks;
use crate::context::Context;
use crate::error::Error;
use crate::raw;
use crate::shim;
use crate::text::{c_path, c_string, from_borrowed, from_owned_message, from_pointer_and_length};
use crate::types::{Invariant, Type};
use crate::values::{Value, ValueMarker};

/// Every exclusive operation on any module gets a number nothing else ever had, so a generation also says which module
/// it is from, and a [`ValueToErase`] can't be used with another module or across an exclusive operation.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

fn new_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// A module, owning its functions, globals and instructions.
///
/// A value from a module borrows it, and deleting from it or running passes on it needs `&mut self`, so a value held
/// across either doesn't compile (D11).
pub struct Module<'context> {
    pub(crate) raw: raw::LLVMModuleRef,
    pub(crate) generation: u64,
    owned: bool,
    marker: Invariant<'context>,
}

impl Drop for Module<'_> {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: this wrapper owns the module, and everything borrowing it has ended.
            unsafe { raw::LLVMDisposeModule(self.raw) };
        }
    }
}

/// A value or block chosen for [`Module::erase`], with no lifetime so that a pass can keep a list of them (D11).
///
/// It remembers the module's generation when it was made, and [`Module::erase`] panics if anything exclusive has
/// happened to the module since, because that could have deleted what it points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueToErase {
    pub(crate) raw: raw::LLVMValueRef,
    generation: u64,
}

/// A handle that survives changes to its module and says whether its value is still there, built on LLVM's `WeakVH`
/// (D11). It carries no module borrow, so it can be held across a delete or a pass run.
pub struct TrackedHandle<'context, T> {
    raw: *mut shim::WeakHandle,
    marker: PhantomData<(Invariant<'context>, fn() -> T)>,
}

impl<T> Drop for TrackedHandle<'_, T> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the handle, made by `lyftar_weak_handle_create` and freed nowhere else.
        unsafe { shim::lyftar_weak_handle_dispose(self.raw) };
    }
}

impl<'context, T: ValueMarker> TrackedHandle<'context, T> {
    /// The value, if it is still there, borrowing `module` from here on.
    ///
    /// # Panics
    /// If the value is in another module (D3).
    #[must_use]
    #[track_caller]
    pub fn get<'module>(&self, module: &'module Module<'context>) -> Option<Value<'context, 'module, T>> {
        // SAFETY: the handle is live until this wrapper drops.
        let value = unsafe { shim::lyftar_weak_handle_get(self.raw) };
        if value.is_null() {
            return None;
        }
        // SAFETY: LLVM nulls the handle when the value goes, so this value is live, and the shim only reads it.
        if !unsafe { shim::lyftar_value_belongs_to_module(value, module.raw) } {
            bad_call(format_args!("get: this handle's value belongs to another module"));
        }
        // SAFETY: live, in this module, and of the kind `T`, as it was when tracked: `WeakVH` doesn't follow a replace.
        Some(unsafe { Value::from_raw(value) })
    }

    /// Whether the value is still there.
    #[must_use]
    pub fn is_present(&self) -> bool {
        // SAFETY: the handle is live until this wrapper drops.
        !unsafe { shim::lyftar_weak_handle_get(self.raw) }.is_null()
    }
}

impl<'context> Module<'context> {
    /// Takes ownership of a raw module.
    ///
    /// # Safety
    /// `raw` must be a live module of this context that nothing else owns.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMModuleRef) -> Self {
        Module { raw, generation: new_generation(), owned: true, marker: PhantomData }
    }

    /// Wraps a raw module someone else owns, as a pass is given one.
    ///
    /// # Safety
    /// `raw` must be a live module of this context, and outlive the wrapper; nothing else may use it meanwhile.
    pub(crate) unsafe fn from_raw_borrowed(raw: raw::LLVMModuleRef) -> Self {
        Module { raw, generation: new_generation(), owned: false, marker: PhantomData }
    }

    /// Gives up ownership of the raw module, for something that takes it over.
    #[must_use]
    pub(crate) fn into_raw(self) -> raw::LLVMModuleRef {
        let raw = self.raw;
        std::mem::forget(self);
        raw
    }

    /// The raw module.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(&self) -> raw::LLVMModuleRef {
        self.raw
    }

    /// Records that something exclusive happened: any [`ValueToErase`] from before it is now stale (D11).
    pub(crate) fn bump_generation(&mut self) {
        self.generation = new_generation();
    }

    /// The context this module belongs to.
    #[must_use]
    pub fn context(&self) -> Context<'context> {
        // SAFETY: the module's context is live for `'context`.
        unsafe { Context::from_raw(raw::LLVMGetModuleContext(self.raw)) }
    }

    /// A copy of this module, in the same context.
    #[must_use]
    pub fn clone_module(&self) -> Module<'context> {
        // SAFETY: the module is live, and the copy is owned by the wrapper from here on.
        unsafe { Module::from_raw(raw::LLVMCloneModule(self.raw)) }
    }

    /// The module's name.
    #[must_use]
    pub fn identifier(&self) -> String {
        let mut length = 0;
        // SAFETY: the module is live; the text is copied right away.
        let text = unsafe { raw::LLVMGetModuleIdentifier(self.raw, &raw mut length) };
        from_pointer_and_length(text, length)
    }

    /// Sets the module's name.
    pub fn set_identifier(&self, identifier: &str) {
        // SAFETY: the module is live, and LLVM reads the length given.
        unsafe { raw::LLVMSetModuleIdentifier(self.raw, identifier.as_ptr().cast(), identifier.len()) };
    }

    /// The name of the source file this module came from.
    #[must_use]
    pub fn source_file_name(&self) -> String {
        let mut length = 0;
        // SAFETY: the module is live; the text is copied right away.
        let text = unsafe { raw::LLVMGetSourceFileName(self.raw, &raw mut length) };
        from_pointer_and_length(text, length)
    }

    /// Sets the name of the source file this module came from.
    pub fn set_source_file_name(&self, name: &str) {
        // SAFETY: the module is live, and LLVM reads the length given.
        unsafe { raw::LLVMSetSourceFileName(self.raw, name.as_ptr().cast(), name.len()) };
    }

    /// The module's data layout text.
    #[must_use]
    pub fn data_layout(&self) -> String {
        // SAFETY: the module is live; the text is copied right away.
        from_borrowed(unsafe { raw::LLVMGetDataLayoutStr(self.raw) })
    }

    /// Sets the module's data layout text.
    ///
    /// # Panics
    /// If the text has a NUL byte inside it, or LLVM can't parse it (D3).
    #[track_caller]
    pub fn set_data_layout(&self, data_layout: &str) {
        let data_layout = c_string(data_layout, "set_data_layout");
        // SAFETY: only reads the text.
        if !unsafe { shim::lyftar_data_layout_is_valid(data_layout.as_ptr()) } {
            bad_call(format_args!("set_data_layout: LLVM can't parse {data_layout:?}"));
        }
        // SAFETY: the module is live, and the text was just checked.
        unsafe { raw::LLVMSetDataLayout(self.raw, data_layout.as_ptr()) };
    }

    /// The module's target triple.
    #[must_use]
    pub fn target(&self) -> String {
        // SAFETY: the module is live; the text is copied right away.
        from_borrowed(unsafe { raw::LLVMGetTarget(self.raw) })
    }

    /// Sets the module's target triple.
    ///
    /// # Panics
    /// If the text has a NUL byte inside it (D3).
    #[track_caller]
    pub fn set_target(&self, triple: &str) {
        let triple = c_string(triple, "set_target");
        // SAFETY: the module is live, and the text is a valid C string for the call.
        unsafe { raw::LLVMSetTarget(self.raw, triple.as_ptr()) };
    }

    /// The module-level inline assembly.
    #[must_use]
    pub fn inline_assembly(&self) -> String {
        let mut length = 0;
        // SAFETY: the module is live; the text is copied right away.
        let text = unsafe { raw::LLVMGetModuleInlineAsm(self.raw, &raw mut length) };
        from_pointer_and_length(text, length)
    }

    /// Sets the module-level inline assembly.
    pub fn set_inline_assembly(&self, assembly: &str) {
        // SAFETY: the module is live, and LLVM reads the length given.
        unsafe { raw::LLVMSetModuleInlineAsm2(self.raw, assembly.as_ptr().cast(), assembly.len()) };
    }

    /// Adds to the module-level inline assembly.
    pub fn append_inline_assembly(&self, assembly: &str) {
        // SAFETY: the module is live, and LLVM reads the length given.
        unsafe { raw::LLVMAppendModuleInlineAsm(self.raw, assembly.as_ptr().cast(), assembly.len()) };
    }

    /// The named struct called `name` in this module's context, if there is one.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn get_named_struct(&self, name: &str) -> Option<Type<'context>> {
        let name = c_string(name, "get_named_struct");
        // SAFETY: the module is live, and the text is a valid C string for the call.
        let found = unsafe { raw::LLVMGetTypeByName(self.raw, name.as_ptr()) };
        // SAFETY: LLVM gave back a live type of this context.
        (!found.is_null()).then(|| unsafe { Type::from_raw(found) })
    }

    /// Runs LLVM's module verifier, and gives back its report if the module is invalid.
    ///
    /// # Errors
    /// The verifier's report.
    pub fn verify(&self) -> Result<(), Error> {
        let mut message = std::ptr::null_mut();
        // SAFETY: the module is live, and the verifier allocates the message for the caller to free.
        let failed = unsafe {
            raw::LLVMVerifyModule(self.raw, raw::LLVMVerifierFailureAction::LLVMReturnStatusAction, &raw mut message)
        };
        let report = Error::from_owned_message(message);
        if failed == 0 { Ok(()) } else { Err(report) }
    }

    /// LLVM's text for this module.
    #[must_use]
    pub fn print_to_string(&self) -> String {
        // SAFETY: the module is live.
        from_owned_message(unsafe { raw::LLVMPrintModuleToString(self.raw) })
    }

    /// Writes LLVM's text for this module to a file.
    ///
    /// # Errors
    /// What stopped the file being written.
    pub fn print_to_file(&self, path: &Path) -> Result<(), Error> {
        let path = c_path(path)?;
        let mut message = std::ptr::null_mut();
        // SAFETY: the module is live, and LLVM allocates the message for the caller to free.
        let failed = unsafe { raw::LLVMPrintModuleToFile(self.raw, path.as_ptr(), &raw mut message) };
        let report = Error::from_owned_message(message);
        if failed == 0 { Ok(()) } else { Err(report) }
    }

    /// The debug metadata version this module's flags say, zero if none.
    #[must_use]
    pub fn debug_metadata_version(&self) -> u32 {
        // SAFETY: the module is live.
        unsafe { raw::LLVMGetModuleDebugMetadataVersion(self.raw) }
    }

    /// Removes all debug information from this module, and says whether there was any.
    pub fn strip_debug_info(&mut self) -> bool {
        // trace: D11
        self.bump_generation();
        // SAFETY: exclusive access means nothing holds a value that this could delete.
        unsafe { raw::LLVMStripModuleDebugInfo(self.raw) != 0 }
    }

    pub(crate) fn entry_is_current(&self, entry: ValueToErase) -> bool {
        checks::entry_is_current(entry.generation, self.generation)
    }

    // trace: D11
    /// Chooses a value for [`Module::erase`].
    ///
    /// Only instructions, functions, global variables, aliases and indirect functions can be erased, and blocks go
    /// through [`Module::block_to_erase`].
    ///
    /// # Panics
    /// If the value is another kind, or it belongs to another module (D3).
    #[must_use]
    #[track_caller]
    pub fn value_to_erase<T>(&self, value: Value<'context, '_, T>) -> ValueToErase {
        // SAFETY: the value is live.
        if !unsafe { shim::lyftar_value_is_erasable(value.raw) } {
            bad_call(format_args!(
                "value_to_erase: {} is not an instruction, function, global variable, alias or indirect function",
                value.print_to_string()
            ));
        }
        self.check_owns("value_to_erase", value);
        ValueToErase { raw: value.raw, generation: self.generation }
    }

    // trace: D11
    /// Chooses a block for [`Module::erase`].
    ///
    /// # Panics
    /// If the block belongs to another module (D3).
    #[must_use]
    #[track_caller]
    pub fn block_to_erase(&self, block: Block<'context, '_>) -> ValueToErase {
        let value = block.as_value();
        self.check_owns("block_to_erase", value);
        ValueToErase { raw: value.raw, generation: self.generation }
    }

    // trace: D11
    /// Erases these values and blocks together.
    ///
    /// Nothing that stays may use anything that goes: replace those uses first. A value can use another in the same
    /// list. A block's erasure also drops it from the phis of the blocks it branched to.
    ///
    /// # Panics
    /// If an entry is stale, because something exclusive has happened to this module since it was chosen, or it came
    /// from another module, or the list has it twice, or something that stays uses it (D3).
    #[track_caller]
    pub fn erase(&mut self, entries: &[ValueToErase]) {
        for entry in entries {
            if !checks::entry_is_current(entry.generation, self.generation) {
                bad_call(format_args!(
                    "erase: this entry is stale: it was chosen before something exclusive happened to this module, \
                     or it came from another module"
                ));
            }
        }
        let mut addresses: Vec<usize> = entries.iter().map(|entry| entry.raw as usize).collect();
        addresses.sort_unstable();
        if !checks::sorted_has_no_duplicates(&addresses) {
            bad_call(format_args!("erase: the list has the same value twice"));
        }
        let values: Vec<raw::LLVMValueRef> = entries.iter().map(|entry| entry.raw).collect();
        // SAFETY: each entry is current, so it is live and in this module, and the list has no duplicates. The shim
        // only reads them.
        let blocker = unsafe { shim::lyftar_erase_find_blocker(values.as_ptr(), values.len()) };
        if !blocker.is_null() {
            // SAFETY: the shim found this among the live values.
            let blocker: Value<'context, '_> = unsafe { Value::from_raw(blocker) };
            bad_call(format_args!("erase: {} is still used by something that stays", blocker.print_to_string()));
        }
        self.bump_generation();
        // SAFETY: just checked what the shim's contract says, and exclusive access means no `Value` is left to dangle.
        unsafe { shim::lyftar_erase_values(values.as_ptr(), values.len()) };
    }

    // trace: D11
    /// A handle on a value that survives changes to this module and says whether the value is still there.
    ///
    /// # Panics
    /// If the value belongs to another module, or it is a constant, which belongs to the context and survives anyway (D3).
    #[must_use]
    #[track_caller]
    pub fn track<T: ValueMarker>(&self, value: Value<'context, '_, T>) -> TrackedHandle<'context, T> {
        self.check_owns("track", value);
        // SAFETY: the value is live; the shim only reads it to decide this.
        if unsafe { shim::lyftar_value_is_context_constant(value.raw) } {
            bad_call(format_args!("track: {} belongs to the context and needs no handle", value.print_to_string()));
        }
        // SAFETY: the value is live; the handle is owned by the wrapper from here on.
        TrackedHandle { raw: unsafe { shim::lyftar_weak_handle_create(value.raw) }, marker: PhantomData }
    }
}
