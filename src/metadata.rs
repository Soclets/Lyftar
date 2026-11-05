//! Metadata. A metadata node made only from strings and other context-level pieces carries `'static` for its module borrow
//! (D11); one that wraps a value from a module borrows that module.

use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::checks;
use crate::context::Context;
use crate::enums::ModuleFlagBehavior;
use crate::module::Module;
use crate::raw;
use crate::text::{c_string, from_pointer_and_length};
use crate::types::Invariant;
use crate::values::{GlobalObjectMarker, MetadataAsValue, Value};

/// A piece of metadata.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Metadata<'context, 'module> {
    pub(crate) raw: raw::LLVMMetadataRef,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl std::fmt::Debug for Metadata<'_, '_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Metadata({:?})", self.raw)
    }
}

/// A kind of metadata on instructions and globals, from [`Context::metadata_kind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MetadataKind(pub(crate) u32);

impl<'context, 'module> Metadata<'context, 'module> {
    /// Wraps raw metadata.
    ///
    /// # Safety
    /// `raw` must be live metadata of this context, and stay live for `'module`.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMMetadataRef) -> Self {
        Metadata { raw, marker: PhantomData }
    }

    /// The raw metadata.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(self) -> raw::LLVMMetadataRef {
        self.raw
    }

    /// LLVM's number for what kind of metadata this is.
    #[must_use]
    pub fn kind_number(self) -> u32 {
        // SAFETY: the metadata is live for `'module`.
        unsafe { raw::LLVMGetMetadataKind(self.raw) }
    }

    /// This metadata wrapped as a value, as an intrinsic's argument takes it.
    #[must_use]
    pub fn as_value(self, context: &Context<'context>) -> Value<'context, 'module, MetadataAsValue> {
        // SAFETY: the metadata is live for `'module`, and belongs to this context.
        unsafe { Value::from_raw(raw::LLVMMetadataAsValue(context.raw, self.raw)) }
    }
}

impl<'context, 'module, T> Value<'context, 'module, T> {
    /// This value wrapped as metadata.
    ///
    /// # Panics
    /// If this value is a block (D3).
    #[must_use]
    #[track_caller]
    pub fn as_metadata(self) -> Metadata<'context, 'module> {
        // SAFETY: the value is live for `'module`, and its kind is only read.
        if unsafe { raw::LLVMGetValueKind(self.raw) } == raw::LLVMValueKind::LLVMBasicBlockValueKind {
            bad_call(format_args!("as_metadata: a block can't be wrapped as metadata"));
        }
        // SAFETY: LLVM's `ValueAsMetadata` takes any other value, and a value that is metadata gives back the metadata.
        unsafe { Metadata::from_raw(raw::LLVMValueAsMetadata(self.raw)) }
    }
}

impl<'context, 'module> Value<'context, 'module, MetadataAsValue> {
    /// The metadata this value wraps.
    #[must_use]
    pub fn metadata(self) -> Metadata<'context, 'module> {
        // SAFETY: the marker guarantees metadata wrapped as a value.
        unsafe { Metadata::from_raw(raw::LLVMValueAsMetadata(self.raw)) }
    }

    /// The text, if this wraps a metadata string.
    #[must_use]
    pub fn metadata_string(self) -> Option<String> {
        // SAFETY: the marker guarantees metadata wrapped as a value, which LLVM asks about.
        unsafe {
            if raw::LLVMIsAMDString(self.raw).is_null() {
                return None;
            }
            let mut length = 0;
            let text = raw::LLVMGetMDString(self.raw, &raw mut length);
            Some(from_pointer_and_length(text, length as usize))
        }
    }

    /// Whether this wraps a metadata node.
    #[must_use]
    pub fn is_metadata_node(self) -> bool {
        // SAFETY: the marker guarantees metadata wrapped as a value, which LLVM asks about.
        unsafe { !raw::LLVMIsAMDNode(self.raw).is_null() }
    }

    /// Whether this wraps a value as metadata.
    #[must_use]
    pub fn is_value_as_metadata(self) -> bool {
        // SAFETY: the marker guarantees metadata wrapped as a value, which LLVM asks about.
        unsafe { !raw::LLVMIsAValueAsMetadata(self.raw).is_null() }
    }

    /// The operands of this metadata node, each as a value, none where the operand is null.
    ///
    /// # Panics
    /// If this doesn't wrap a metadata node (D3).
    #[must_use]
    #[track_caller]
    pub fn node_operands(self) -> Vec<Option<Value<'context, 'module>>> {
        if !self.is_metadata_node() {
            bad_call(format_args!("node_operands: {} is not a metadata node", self.describe()));
        }
        // SAFETY: just checked; the buffer has room for all the operands.
        unsafe {
            let count = raw::LLVMGetMDNodeNumOperands(self.raw);
            let mut operands = vec![std::ptr::null_mut(); count as usize];
            raw::LLVMGetMDNodeOperands(self.raw, operands.as_mut_ptr());
            operands.into_iter().map(|operand| (!operand.is_null()).then(|| Value::from_raw(operand))).collect()
        }
    }

    // trace: D11
    /// Replaces the operand at `index` of this metadata node.
    ///
    /// The replacement belongs to the context, because a node made only from the context can be held across a delete: a
    /// value from a module in it would then be reachable after that value goes.
    ///
    /// # Panics
    /// If this doesn't wrap a resolved metadata node, or it has no such operand (D3).
    #[track_caller]
    pub fn replace_node_operand(self, index: u32, replacement: Metadata<'context, 'static>) {
        // SAFETY: the marker guarantees a live wrapped value. LLVM can delete an unresolved node when one of its operands
        // changes, which would leave other handles on it dangling, so those are refused.
        if !unsafe { crate::shim::lyftar_metadata_node_can_change_operand(raw::LLVMValueAsMetadata(self.raw)) } {
            bad_call(format_args!("replace_node_operand: {} is not a resolved metadata node", self.describe()));
        }
        // SAFETY: just checked.
        let count = unsafe { raw::LLVMGetMDNodeNumOperands(self.raw) };
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("replace_node_operand: index {index} is out of bounds for {count} operands"));
        }
        // SAFETY: both were just checked.
        unsafe { raw::LLVMReplaceMDNodeOperandWith(self.raw, index, replacement.raw) };
    }
}

impl<'context> Context<'context> {
    /// The metadata kind called `name`, such as `tbaa` or `llvm.loop`, registering it if it is new.
    #[must_use]
    #[track_caller]
    pub fn metadata_kind(&self, name: &str) -> MetadataKind {
        let length = crate::types::length_as_u32(name.len(), "metadata_kind");
        // SAFETY: LLVM reads `length` bytes.
        MetadataKind(unsafe { raw::LLVMGetMDKindIDInContext(self.raw, name.as_ptr().cast(), length) })
    }

    /// A metadata string.
    #[must_use]
    pub fn metadata_string(&self, text: &str) -> Metadata<'context, 'static> {
        // SAFETY: LLVM reads `text.len()` bytes; the string belongs to the context.
        unsafe { Metadata::from_raw(raw::LLVMMDStringInContext2(self.raw, text.as_ptr().cast(), text.len())) }
    }

    /// A metadata node.
    #[must_use]
    pub fn metadata_node<'module>(&self, operands: &[Metadata<'context, 'module>]) -> Metadata<'context, 'module> {
        // SAFETY: LLVM only reads the operands; `Metadata` has the layout of `LLVMMetadataRef`.
        unsafe {
            Metadata::from_raw(raw::LLVMMDNodeInContext2(self.raw, operands.as_ptr().cast_mut().cast(), operands.len()))
        }
    }

    /// The synchronization scope called `name`, registering it if it is new.
    #[must_use]
    pub fn synchronization_scope(&self, name: &str) -> crate::instructions::SynchronizationScope {
        // SAFETY: LLVM reads `name.len()` bytes.
        crate::instructions::SynchronizationScope(unsafe {
            raw::LLVMGetSyncScopeID(self.raw, name.as_ptr().cast(), name.len())
        })
    }
}

/// Copies metadata entries and frees LLVM's list.
///
/// # Safety
/// `entries` must be a list of `count` entries from LLVM, not used afterwards. It may be null if `count` is zero.
pub(crate) unsafe fn copy_entries<'context, 'module>(
    entries: *mut raw::LLVMValueMetadataEntry,
    count: usize,
) -> Vec<(MetadataKind, Metadata<'context, 'module>)> {
    let mut copied = Vec::with_capacity(count);
    if entries.is_null() {
        return copied;
    }
    // SAFETY: the caller promises the list has `count` entries; it's freed once, after the copying.
    unsafe {
        for index in 0..count {
            // `count` came from a list LLVM indexes by `unsigned`.
            #[allow(clippy::cast_possible_truncation)]
            let index = index as u32;
            let kind = raw::LLVMValueMetadataEntriesGetKind(entries, index);
            let metadata = raw::LLVMValueMetadataEntriesGetMetadata(entries, index);
            copied.push((MetadataKind(kind), Metadata::from_raw(metadata)));
        }
        raw::LLVMDisposeValueMetadataEntries(entries);
    }
    copied
}

impl<'context, 'module, T: GlobalObjectMarker> Value<'context, 'module, T> {
    /// Sets this global's metadata of a kind, replacing what it had.
    pub fn set_global_metadata(self, kind: MetadataKind, metadata: Metadata<'context, 'module>) {
        // SAFETY: the marker guarantees a global object.
        unsafe { raw::LLVMGlobalSetMetadata(self.raw, kind.0, metadata.raw) };
    }

    /// Adds to this global's metadata of a kind.
    pub fn add_global_metadata(self, kind: MetadataKind, metadata: Metadata<'context, 'module>) {
        // SAFETY: the marker guarantees a global object.
        unsafe { raw::LLVMGlobalAddMetadata(self.raw, kind.0, metadata.raw) };
    }

    /// Removes this global's metadata of a kind.
    pub fn erase_global_metadata(self, kind: MetadataKind) {
        // SAFETY: the marker guarantees a global object.
        unsafe { raw::LLVMGlobalEraseMetadata(self.raw, kind.0) };
    }

    /// Removes all of this global's metadata.
    pub fn clear_global_metadata(self) {
        // SAFETY: the marker guarantees a global object.
        unsafe { raw::LLVMGlobalClearMetadata(self.raw) };
    }

    /// All of this global's metadata.
    #[must_use]
    pub fn all_global_metadata(self) -> Vec<(MetadataKind, Metadata<'context, 'module>)> {
        let mut count = 0;
        // SAFETY: the marker guarantees a global object; the entries are copied and freed.
        unsafe { copy_entries(raw::LLVMGlobalCopyAllMetadata(self.raw, &raw mut count), count) }
    }
}

/// A module's named metadata, such as `llvm.module.flags`.
#[derive(Clone, Copy)]
pub struct NamedMetadata<'context, 'module> {
    raw: raw::LLVMNamedMDNodeRef,
    module: raw::LLVMModuleRef,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl<'context, 'module> NamedMetadata<'context, 'module> {
    /// Wraps raw named metadata.
    ///
    /// # Safety
    /// `raw` must be live named metadata in `module`, staying live for `'module`.
    #[must_use]
    pub unsafe fn from_raw(raw: raw::LLVMNamedMDNodeRef, module: raw::LLVMModuleRef) -> Self {
        NamedMetadata { raw, module, marker: PhantomData }
    }

    /// The raw named metadata.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(self) -> raw::LLVMNamedMDNodeRef {
        self.raw
    }

    /// This metadata's name.
    #[must_use]
    pub fn name(self) -> String {
        let mut length = 0;
        // SAFETY: live for `'module`; the text is copied right away.
        let text = unsafe { raw::LLVMGetNamedMetadataName(self.raw, &raw mut length) };
        from_pointer_and_length(text, length)
    }

    /// The next named metadata in the module.
    #[must_use]
    pub fn next(self) -> Option<Self> {
        // SAFETY: live for `'module`.
        let next = unsafe { raw::LLVMGetNextNamedMetadata(self.raw) };
        (!next.is_null()).then_some(NamedMetadata { raw: next, ..self })
    }

    /// The previous named metadata in the module.
    #[must_use]
    pub fn previous(self) -> Option<Self> {
        // SAFETY: live for `'module`.
        let previous = unsafe { raw::LLVMGetPreviousNamedMetadata(self.raw) };
        (!previous.is_null()).then_some(NamedMetadata { raw: previous, ..self })
    }

    /// This metadata's operands, each a metadata node wrapped as a value.
    #[must_use]
    pub fn operands(self) -> Vec<Value<'context, 'module, MetadataAsValue>> {
        let name = c_string(&self.name(), "operands");
        // SAFETY: this named metadata is in that module, so the name finds it; the buffer has room for all.
        unsafe {
            let count = raw::LLVMGetNamedMetadataNumOperands(self.module, name.as_ptr());
            let mut operands = vec![std::ptr::null_mut(); count as usize];
            raw::LLVMGetNamedMetadataOperands(self.module, name.as_ptr(), operands.as_mut_ptr());
            operands.into_iter().map(|operand| Value::from_raw(operand)).collect()
        }
    }

    /// Adds a metadata node as an operand.
    ///
    /// # Panics
    /// If the value doesn't wrap a metadata node (D3).
    #[track_caller]
    pub fn add_operand(self, node: Value<'context, 'module, MetadataAsValue>) {
        if !node.is_metadata_node() {
            bad_call(format_args!("add_operand: {} is not a metadata node", node.describe()));
        }
        let name = c_string(&self.name(), "add_operand");
        // SAFETY: just checked; this named metadata is in that module.
        unsafe { raw::LLVMAddNamedMetadataOperand(self.module, name.as_ptr(), node.raw) };
    }
}

/// One of a module's flags.
pub struct ModuleFlag<'context, 'module> {
    /// What the linker does when two modules set it.
    pub behavior: ModuleFlagBehavior,
    /// Its name.
    pub key: String,
    /// Its value.
    pub value: Metadata<'context, 'module>,
}

impl<'context> Module<'context> {
    /// The named metadata called `name`, if there is one.
    #[must_use]
    pub fn named_metadata<'module>(&'module self, name: &str) -> Option<NamedMetadata<'context, 'module>> {
        // SAFETY: the module is live; LLVM reads `name.len()` bytes.
        let found = unsafe { raw::LLVMGetNamedMetadata(self.raw, name.as_ptr().cast(), name.len()) };
        (!found.is_null()).then_some(NamedMetadata { raw: found, module: self.raw, marker: PhantomData })
    }

    /// The named metadata called `name`, adding it if it isn't there.
    #[must_use]
    pub fn get_or_insert_named_metadata<'module>(&'module self, name: &str) -> NamedMetadata<'context, 'module> {
        // SAFETY: the module is live; LLVM reads `name.len()` bytes.
        let raw = unsafe { raw::LLVMGetOrInsertNamedMetadata(self.raw, name.as_ptr().cast(), name.len()) };
        NamedMetadata { raw, module: self.raw, marker: PhantomData }
    }

    /// The module's first named metadata.
    #[must_use]
    pub fn first_named_metadata<'module>(&'module self) -> Option<NamedMetadata<'context, 'module>> {
        // SAFETY: the module is live.
        let first = unsafe { raw::LLVMGetFirstNamedMetadata(self.raw) };
        (!first.is_null()).then_some(NamedMetadata { raw: first, module: self.raw, marker: PhantomData })
    }

    /// The module's last named metadata.
    #[must_use]
    pub fn last_named_metadata<'module>(&'module self) -> Option<NamedMetadata<'context, 'module>> {
        // SAFETY: the module is live.
        let last = unsafe { raw::LLVMGetLastNamedMetadata(self.raw) };
        (!last.is_null()).then_some(NamedMetadata { raw: last, module: self.raw, marker: PhantomData })
    }

    /// All the module's flags.
    #[must_use]
    pub fn module_flags<'module>(&'module self) -> Vec<ModuleFlag<'context, 'module>> {
        let mut count = 0;
        // SAFETY: the module is live; LLVM gives back a list of `count` entries, read here and then freed.
        unsafe {
            let entries = raw::LLVMCopyModuleFlagsMetadata(self.raw, &raw mut count);
            if entries.is_null() {
                return Vec::new();
            }
            let mut flags = Vec::with_capacity(count);
            for index in 0..count {
                // `count` came from a list LLVM indexes by `unsigned`.
                #[allow(clippy::cast_possible_truncation)]
                let index = index as u32;
                let mut length = 0;
                let key = raw::LLVMModuleFlagEntriesGetKey(entries, index, &raw mut length);
                flags.push(ModuleFlag {
                    behavior: ModuleFlagBehavior::from_raw(raw::LLVMModuleFlagEntriesGetFlagBehavior(entries, index)),
                    key: from_pointer_and_length(key, length),
                    value: Metadata::from_raw(raw::LLVMModuleFlagEntriesGetMetadata(entries, index)),
                });
            }
            raw::LLVMDisposeModuleFlagsMetadata(entries);
            flags
        }
    }

    /// The module flag called `key`, if there is one.
    #[must_use]
    pub fn module_flag<'module>(&'module self, key: &str) -> Option<Metadata<'context, 'module>> {
        // SAFETY: the module is live; LLVM reads `key.len()` bytes.
        let found = unsafe { raw::LLVMGetModuleFlag(self.raw, key.as_ptr().cast(), key.len()) };
        // SAFETY: LLVM gave back live metadata.
        (!found.is_null()).then(|| unsafe { Metadata::from_raw(found) })
    }

    /// Adds a module flag.
    pub fn add_module_flag(&self, behavior: ModuleFlagBehavior, key: &str, value: Metadata<'context, '_>) {
        // SAFETY: the module is live; LLVM reads `key.len()` bytes.
        unsafe { raw::LLVMAddModuleFlag(self.raw, behavior.to_raw(), key.as_ptr().cast(), key.len(), value.raw) };
    }
}
