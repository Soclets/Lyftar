//! Functions, global variables, aliases and indirect functions.

use std::marker::PhantomData;

use crate::attributes::{Attribute, AttributeKind, AttributeLocation};
use crate::bad_call::bad_call;
use crate::basic_block::BasicBlock;
use crate::checks;
use crate::context::Context;
use crate::enums::{DllStorageClass, Linkage, ThreadLocalMode, UnnamedAddress, Visibility};
use crate::module::Module;
use crate::raw;
use crate::shim;
use crate::text::{c_string, from_borrowed, from_pointer_and_length};
use crate::types::{Dynamic, Invariant, Type, check_address_space, length_as_u32};
use crate::values::{
    Argument, Constant, Function, GlobalAlias, GlobalIndirectFunction, GlobalObjectMarker, GlobalValueMarker,
    GlobalVariable, SpecificFunction, Value, VariadicFunction,
};

/// A calling convention: one of the named ones here, or `CallingConvention(n)` for LLVM's number `n`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CallingConvention(pub u32);

impl CallingConvention {
    /// `ccc`.
    pub const C: Self = CallingConvention(raw::LLVMCallConv::LLVMCCallConv.0);
    /// `fastcc`.
    pub const FAST: Self = CallingConvention(raw::LLVMCallConv::LLVMFastCallConv.0);
    /// `coldcc`.
    pub const COLD: Self = CallingConvention(raw::LLVMCallConv::LLVMColdCallConv.0);
    /// `tailcc`, which LLVM numbers 18.
    pub const TAIL: Self = CallingConvention(18);

    /// LLVM's number for this convention.
    ///
    /// # Panics
    /// If LLVM can't hold it, past 1023 (D3).
    #[track_caller]
    pub(crate) fn checked_number(self, operation: &str) -> u32 {
        if !checks::calling_convention_is_valid(self.0) {
            bad_call(format_args!("{operation}: calling convention {} is past LLVM's 1023", self.0));
        }
        self.0
    }
}

impl<'context, 'module, T: GlobalValueMarker> Value<'context, 'module, T> {
    /// The module this global is in.
    #[must_use]
    pub fn parent_module(self) -> ModuleRef<'context, 'module> {
        // SAFETY: Lyftar only hands out globals in a module, which lives for `'module`.
        ModuleRef { raw: unsafe { raw::LLVMGetGlobalParent(self.raw) }, marker: PhantomData }
    }

    /// Whether this global has no body or initializer here.
    #[must_use]
    pub fn is_declaration(self) -> bool {
        // SAFETY: the marker guarantees a global.
        unsafe { raw::LLVMIsDeclaration(self.raw) != 0 }
    }

    /// This global's linkage.
    #[must_use]
    pub fn linkage(self) -> Linkage {
        // SAFETY: the marker guarantees a global.
        Linkage::from_raw(unsafe { raw::LLVMGetLinkage(self.raw) })
    }

    /// Sets this global's linkage.
    pub fn set_linkage(self, linkage: Linkage) {
        // SAFETY: the marker guarantees a global.
        unsafe { raw::LLVMSetLinkage(self.raw, linkage.to_raw()) };
    }

    /// This global's visibility.
    #[must_use]
    pub fn visibility(self) -> Visibility {
        // SAFETY: the marker guarantees a global.
        Visibility::from_raw(unsafe { raw::LLVMGetVisibility(self.raw) })
    }

    /// Sets this global's visibility.
    ///
    /// # Panics
    /// If its linkage is local and the visibility isn't default, which LLVM forbids (D3).
    #[track_caller]
    pub fn set_visibility(self, visibility: Visibility) {
        if visibility != Visibility::Default && matches!(self.linkage(), Linkage::Internal | Linkage::Private) {
            bad_call(format_args!("set_visibility: a local global can only have default visibility"));
        }
        // SAFETY: the marker guarantees a global, and the pair was just checked.
        unsafe { raw::LLVMSetVisibility(self.raw, visibility.to_raw()) };
    }

    /// This global's Windows DLL storage class.
    #[must_use]
    pub fn dll_storage_class(self) -> DllStorageClass {
        // SAFETY: the marker guarantees a global.
        DllStorageClass::from_raw(unsafe { raw::LLVMGetDLLStorageClass(self.raw) })
    }

    /// Sets this global's Windows DLL storage class.
    pub fn set_dll_storage_class(self, class: DllStorageClass) {
        // SAFETY: the marker guarantees a global.
        unsafe { raw::LLVMSetDLLStorageClass(self.raw, class.to_raw()) };
    }

    /// Whether this global's address is significant.
    #[must_use]
    pub fn unnamed_address(self) -> UnnamedAddress {
        // SAFETY: the marker guarantees a global.
        UnnamedAddress::from_raw(unsafe { raw::LLVMGetUnnamedAddress(self.raw) })
    }

    /// Sets whether this global's address is significant.
    pub fn set_unnamed_address(self, value: UnnamedAddress) {
        // SAFETY: the marker guarantees a global.
        unsafe { raw::LLVMSetUnnamedAddress(self.raw, value.to_raw()) };
    }

    /// The type of what this global holds, or of the function it is.
    #[must_use]
    pub fn value_type(self) -> Type<'context> {
        // SAFETY: the marker guarantees a global.
        unsafe { Type::from_raw(raw::LLVMGlobalGetValueType(self.raw)) }
    }

    /// This global as a pointer value, whose address space is only known when the line runs (D31). `checked_cast` gives a
    /// specific one.
    #[must_use]
    pub fn as_pointer(self) -> Value<'context, 'module> {
        // SAFETY: any value can be held as a dynamic one, and this one's own LLVM type is a pointer.
        unsafe { Value::from_raw(self.raw) }
    }
}

impl<'module, T: GlobalObjectMarker> Value<'_, 'module, T> {
    /// This global's section, if it has one.
    #[must_use]
    pub fn section(self) -> Option<String> {
        // SAFETY: the marker guarantees a global object, and the name is copied right away.
        let section = unsafe { raw::LLVMGetSection(self.raw) };
        (!section.is_null()).then(|| from_borrowed(section))
    }

    /// Sets this global's section.
    ///
    /// # Panics
    /// If `section` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn set_section(self, section: &str) {
        let section = c_string(section, "set_section");
        // SAFETY: the marker guarantees a global object, and the name is a valid C string for the call.
        unsafe { raw::LLVMSetSection(self.raw, section.as_ptr()) };
    }

    /// This global's COMDAT, if it has one.
    #[must_use]
    pub fn comdat(self) -> Option<Comdat<'module>> {
        // SAFETY: the marker guarantees a global object.
        let comdat = unsafe { raw::LLVMGetComdat(self.raw) };
        (!comdat.is_null()).then_some(Comdat { raw: comdat, marker: PhantomData })
    }

    /// Sets this global's COMDAT.
    pub fn set_comdat(self, comdat: Comdat<'module>) {
        // SAFETY: the marker guarantees a global object, and the COMDAT's lifetime is this module's.
        unsafe { raw::LLVMSetComdat(self.raw, comdat.raw) };
    }
}

/// A module reached from something in it, for asking about it. It can't delete or run passes, which need `&mut Module`.
#[derive(Clone, Copy)]
pub struct ModuleRef<'context, 'module> {
    pub(crate) raw: raw::LLVMModuleRef,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl<'context> ModuleRef<'context, '_> {
    /// The raw module.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(self) -> raw::LLVMModuleRef {
        self.raw
    }

    /// The module's name.
    #[must_use]
    pub fn identifier(self) -> String {
        let mut length = 0;
        // SAFETY: the module lives for `'module`; the text is copied right away.
        let text = unsafe { raw::LLVMGetModuleIdentifier(self.raw, &raw mut length) };
        from_pointer_and_length(text, length)
    }

    /// The module's context.
    #[must_use]
    pub fn context(self) -> Context<'context> {
        // SAFETY: the module's context lives for `'context`.
        unsafe { Context::from_raw(raw::LLVMGetModuleContext(self.raw)) }
    }
}

impl PartialEq for ModuleRef<'_, '_> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

/// A COMDAT: a group of globals that a linker keeps or drops together.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Comdat<'module> {
    raw: raw::LLVMComdatRef,
    marker: PhantomData<&'module ()>,
}

/// How a linker picks among COMDATs of one name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ComdatSelectionKind {
    /// Any of them.
    Any,
    /// They must have the same contents.
    ExactMatch,
    /// The largest.
    Largest,
    /// There must be only one.
    NoDeduplicate,
    /// They must have the same size.
    SameSize,
    /// A value this Lyftar doesn't name.
    Unknown(u32),
}

impl Comdat<'_> {
    /// How a linker picks among COMDATs of this name.
    #[must_use]
    pub fn selection_kind(self) -> ComdatSelectionKind {
        use raw::LLVMComdatSelectionKind as K;
        // SAFETY: the COMDAT lives as long as its module.
        match unsafe { raw::LLVMGetComdatSelectionKind(self.raw) } {
            K::LLVMAnyComdatSelectionKind => ComdatSelectionKind::Any,
            K::LLVMExactMatchComdatSelectionKind => ComdatSelectionKind::ExactMatch,
            K::LLVMLargestComdatSelectionKind => ComdatSelectionKind::Largest,
            K::LLVMNoDeduplicateComdatSelectionKind => ComdatSelectionKind::NoDeduplicate,
            K::LLVMSameSizeComdatSelectionKind => ComdatSelectionKind::SameSize,
            K(other) => ComdatSelectionKind::Unknown(other),
        }
    }

    /// Sets how a linker picks among COMDATs of this name.
    pub fn set_selection_kind(self, kind: ComdatSelectionKind) {
        use raw::LLVMComdatSelectionKind as K;
        let raw_kind = match kind {
            ComdatSelectionKind::Any => K::LLVMAnyComdatSelectionKind,
            ComdatSelectionKind::ExactMatch => K::LLVMExactMatchComdatSelectionKind,
            ComdatSelectionKind::Largest => K::LLVMLargestComdatSelectionKind,
            ComdatSelectionKind::NoDeduplicate => K::LLVMNoDeduplicateComdatSelectionKind,
            ComdatSelectionKind::SameSize => K::LLVMSameSizeComdatSelectionKind,
            ComdatSelectionKind::Unknown(other) => K(other),
        };
        // SAFETY: the COMDAT lives as long as its module.
        unsafe { raw::LLVMSetComdatSelectionKind(self.raw, raw_kind) };
    }
}

/// Gives back the next item in a list of globals, or none.
fn next_global<'context, 'module, T>(raw_value: raw::LLVMValueRef) -> Option<Value<'context, 'module, T>> {
    // SAFETY: the caller got `raw_value` from LLVM's own list of a module that lives for `'module`.
    (!raw_value.is_null()).then(|| unsafe { Value::from_raw(raw_value) })
}

/// Declares an iterator over one of a module's lists of globals.
macro_rules! global_list {
    ($(#[$meta:meta])* $iterator:ident, $marker:ident, $method:ident, $first:ident, $next:ident) => {
        $(#[$meta])*
        pub struct $iterator<'context, 'module> {
            next: Option<Value<'context, 'module, $marker>>,
        }

        impl<'context, 'module> Iterator for $iterator<'context, 'module> {
            type Item = Value<'context, 'module, $marker>;

            fn next(&mut self) -> Option<Self::Item> {
                let current = self.next?;
                // SAFETY: `current` is in this list, and nothing can delete it while `'module` lasts.
                self.next = next_global(unsafe { raw::$next(current.raw) });
                Some(current)
            }
        }

        impl<'context> Module<'context> {
            $(#[$meta])*
            #[must_use]
            pub fn $method<'module>(&'module self) -> $iterator<'context, 'module> {
                // SAFETY: the module is live.
                $iterator { next: next_global(unsafe { raw::$first(self.raw) }) }
            }
        }
    };
}

global_list!(
    /// The functions in a module, in order.
    Functions, Function, functions, LLVMGetFirstFunction, LLVMGetNextFunction
);
global_list!(
    /// The global variables in a module, in order.
    GlobalVariables, GlobalVariable, global_variables, LLVMGetFirstGlobal, LLVMGetNextGlobal
);
global_list!(
    /// The aliases in a module, in order.
    GlobalAliases, GlobalAlias, global_aliases, LLVMGetFirstGlobalAlias, LLVMGetNextGlobalAlias
);
global_list!(
    /// The indirect functions in a module, in order.
    GlobalIndirectFunctions, GlobalIndirectFunction, global_indirect_functions, LLVMGetFirstGlobalIFunc,
    LLVMGetNextGlobalIFunc
);

impl<'context> Module<'context> {
    /// Adds a function of the given function type.
    ///
    /// # Panics
    /// If `function_type` isn't a function type, or `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn add_function<'module>(
        &'module self,
        name: &str,
        function_type: Type<'context, Dynamic>,
    ) -> Value<'context, 'module, Function> {
        check_function_type("add_function", function_type);
        let name = c_string(name, "add_function");
        // SAFETY: the module is live, the type was just checked, and `name` is a valid C string for the call.
        unsafe { Value::from_raw(raw::LLVMAddFunction(self.raw, name.as_ptr(), function_type.raw)) }
    }

    /// The function called `name`, adding a declaration if it isn't there. If something of that name but another type
    /// is there, the result is that other function, whose type [`Value::value_type`] says.
    ///
    /// # Panics
    /// If `function_type` isn't a function type (D3).
    #[must_use]
    #[track_caller]
    pub fn get_or_insert_function<'module>(
        &'module self,
        name: &str,
        function_type: Type<'context, Dynamic>,
    ) -> Value<'context, 'module, Function> {
        check_function_type("get_or_insert_function", function_type);
        // SAFETY: the module is live, the type was just checked, and LLVM reads `name.len()` bytes.
        let found =
            unsafe { raw::LLVMGetOrInsertFunction(self.raw, name.as_ptr().cast(), name.len(), function_type.raw) };
        // LLVM hands back whatever global already has that name, which needn't be a function.
        // SAFETY: LLVM just gave back a live global.
        if unsafe { raw::LLVMIsAFunction(found) }.is_null() {
            bad_call(format_args!("get_or_insert_function: {name} is already a global that isn't a function"));
        }
        // SAFETY: just checked to be a function.
        unsafe { Value::from_raw(found) }
    }

    /// The function called `name`, if there is one.
    #[must_use]
    pub fn get_function<'module>(&'module self, name: &str) -> Option<Value<'context, 'module, Function>> {
        // SAFETY: the module is live, and LLVM reads `name.len()` bytes.
        next_global(unsafe { raw::LLVMGetNamedFunctionWithLength(self.raw, name.as_ptr().cast(), name.len()) })
    }

    /// The module's last function.
    #[must_use]
    pub fn last_function<'module>(&'module self) -> Option<Value<'context, 'module, Function>> {
        // SAFETY: the module is live.
        next_global(unsafe { raw::LLVMGetLastFunction(self.raw) })
    }

    /// Adds a global variable holding a value of the given type.
    ///
    /// # Panics
    /// If that type can't be a global's, or `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn add_global<'module, T>(
        &'module self,
        name: &str,
        global_type: Type<'context, T>,
    ) -> Value<'context, 'module, GlobalVariable> {
        self.add_global_in_address_space(name, global_type, 0)
    }

    // trace: D33
    /// Adds a function whose signature is written into the Rust code, as in
    /// `add_specific_function::<Function<Void, (Pointer,)>>("name")`, so calls to it are checked at `cargo build`.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn add_specific_function<'module, F: SpecificFunction>(
        &'module self,
        name: &str,
    ) -> Value<'context, 'module, F> {
        let name = c_string(name, "add_specific_function");
        // SAFETY: the module is live, its context gives the type, and `name` is a valid C string for the call.
        unsafe { Value::from_raw(raw::LLVMAddFunction(self.raw, name.as_ptr(), F::raw_function_type(self.context()))) }
    }

    /// Adds a global variable in an address space.
    ///
    /// # Panics
    /// If that type can't be a global's, or `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn add_global_in_address_space<'module, T>(
        &'module self,
        name: &str,
        global_type: Type<'context, T>,
        address_space: u32,
    ) -> Value<'context, 'module, GlobalVariable> {
        check_address_space("add_global", address_space);
        // SAFETY: only reads the type.
        if !unsafe { shim::lyftar_type_is_valid_global(global_type.raw) } {
            bad_call(format_args!("add_global: {} can't be a global's type", global_type.describe()));
        }
        let name = c_string(name, "add_global");
        // SAFETY: the module is live, the type was just checked, and `name` is a valid C string for the call.
        unsafe {
            Value::from_raw(raw::LLVMAddGlobalInAddressSpace(self.raw, global_type.raw, name.as_ptr(), address_space))
        }
    }

    /// The global variable called `name`, if there is one.
    #[must_use]
    pub fn get_global<'module>(&'module self, name: &str) -> Option<Value<'context, 'module, GlobalVariable>> {
        // SAFETY: the module is live, and LLVM reads `name.len()` bytes.
        next_global(unsafe { raw::LLVMGetNamedGlobalWithLength(self.raw, name.as_ptr().cast(), name.len()) })
    }

    /// The module's last global variable.
    #[must_use]
    pub fn last_global<'module>(&'module self) -> Option<Value<'context, 'module, GlobalVariable>> {
        // SAFETY: the module is live.
        next_global(unsafe { raw::LLVMGetLastGlobal(self.raw) })
    }

    /// Adds an alias of a global.
    ///
    /// # Panics
    /// If the aliasee isn't a constant pointer in this module, or `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn add_alias<'module, T>(
        &'module self,
        value_type: Type<'context>,
        address_space: u32,
        aliasee: Value<'context, 'module, T>,
        name: &str,
    ) -> Value<'context, 'module, GlobalAlias> {
        check_address_space("add_alias", address_space);
        check_constant_pointer("add_alias", aliasee);
        self.check_owns("add_alias", aliasee);
        let name = c_string(name, "add_alias");
        // SAFETY: the module is live, the aliasee was just checked, and `name` is a valid C string for the call.
        unsafe {
            Value::from_raw(raw::LLVMAddAlias2(self.raw, value_type.raw, address_space, aliasee.raw, name.as_ptr()))
        }
    }

    /// The alias called `name`, if there is one.
    #[must_use]
    pub fn get_alias<'module>(&'module self, name: &str) -> Option<Value<'context, 'module, GlobalAlias>> {
        // SAFETY: the module is live, and LLVM reads `name.len()` bytes.
        next_global(unsafe { raw::LLVMGetNamedGlobalAlias(self.raw, name.as_ptr().cast(), name.len()) })
    }

    /// The module's last alias.
    #[must_use]
    pub fn last_alias<'module>(&'module self) -> Option<Value<'context, 'module, GlobalAlias>> {
        // SAFETY: the module is live.
        next_global(unsafe { raw::LLVMGetLastGlobalAlias(self.raw) })
    }

    /// Adds an indirect function, whose resolver picks the implementation at load time.
    ///
    /// # Panics
    /// If `function_type` isn't a function type, or the resolver isn't a constant pointer in this module (D3).
    #[must_use]
    #[track_caller]
    pub fn add_global_indirect_function<'module, T>(
        &'module self,
        name: &str,
        function_type: Type<'context>,
        address_space: u32,
        resolver: Value<'context, 'module, T>,
    ) -> Value<'context, 'module, GlobalIndirectFunction> {
        check_address_space("add_global_indirect_function", address_space);
        check_function_type("add_global_indirect_function", function_type);
        check_constant_pointer("add_global_indirect_function", resolver);
        self.check_owns("add_global_indirect_function", resolver);
        // SAFETY: the module is live, the rest were just checked, and LLVM reads `name.len()` bytes.
        unsafe {
            Value::from_raw(raw::LLVMAddGlobalIFunc(
                self.raw,
                name.as_ptr().cast(),
                name.len(),
                function_type.raw,
                address_space,
                resolver.raw,
            ))
        }
    }

    /// The indirect function called `name`, if there is one.
    #[must_use]
    pub fn get_global_indirect_function<'module>(
        &'module self,
        name: &str,
    ) -> Option<Value<'context, 'module, GlobalIndirectFunction>> {
        // SAFETY: the module is live, and LLVM reads `name.len()` bytes.
        next_global(unsafe { raw::LLVMGetNamedGlobalIFunc(self.raw, name.as_ptr().cast(), name.len()) })
    }

    /// The module's last indirect function.
    #[must_use]
    pub fn last_global_indirect_function<'module>(
        &'module self,
    ) -> Option<Value<'context, 'module, GlobalIndirectFunction>> {
        // SAFETY: the module is live.
        next_global(unsafe { raw::LLVMGetLastGlobalIFunc(self.raw) })
    }

    /// The COMDAT called `name`, adding it if it isn't there.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn get_or_insert_comdat<'module>(&'module self, name: &str) -> Comdat<'module> {
        let name = c_string(name, "get_or_insert_comdat");
        // SAFETY: the module is live, and `name` is a valid C string for the call.
        Comdat { raw: unsafe { raw::LLVMGetOrInsertComdat(self.raw, name.as_ptr()) }, marker: PhantomData }
    }

    /// An intrinsic's name with the overload types spelled in, as a declaration has it.
    ///
    /// # Panics
    /// If the types don't fit the intrinsic (D3).
    #[must_use]
    #[track_caller]
    pub fn intrinsic_overloaded_name(
        &self,
        intrinsic: crate::intrinsics::Intrinsic,
        overload_types: &[Type<'context>],
    ) -> String {
        intrinsic.check_overloads("intrinsic_overloaded_name", overload_types);
        let mut length = 0;
        // SAFETY: the overloads were just checked and LLVM only reads them; it allocates the name for the caller to free.
        let text = unsafe {
            raw::LLVMIntrinsicCopyOverloadedName2(
                self.raw,
                intrinsic.0,
                overload_types.as_ptr().cast_mut().cast(),
                overload_types.len(),
                &raw mut length,
            )
        };
        crate::text::from_owned_message(text)
    }

    /// The declaration of an intrinsic, with the types it is overloaded on.
    ///
    /// # Panics
    /// If the intrinsic is overloaded on a different number of types (D3).
    #[must_use]
    #[track_caller]
    pub fn intrinsic_declaration<'module>(
        &'module self,
        intrinsic: crate::intrinsics::Intrinsic,
        overload_types: &[Type<'context>],
    ) -> Value<'context, 'module, Function> {
        intrinsic.check_overloads("intrinsic_declaration", overload_types);
        // SAFETY: the module is live, and the overloads were just checked. LLVM only reads them.
        unsafe {
            Value::from_raw(raw::LLVMGetIntrinsicDeclaration(
                self.raw,
                intrinsic.0,
                overload_types.as_ptr().cast_mut().cast(),
                overload_types.len(),
            ))
        }
    }

    #[track_caller]
    pub(crate) fn check_owns<T>(&self, operation: &str, value: Value<'context, '_, T>) {
        // SAFETY: both are live; the shim only reads them.
        if !unsafe { shim::lyftar_value_belongs_to_module(value.raw, self.raw) } {
            bad_call(format_args!("{operation}: {} belongs to another module", value.describe()));
        }
    }
}

#[track_caller]
fn check_function_type(operation: &str, function_type: Type<'_>) {
    if function_type.raw_kind() != raw::LLVMTypeKind::LLVMFunctionTypeKind {
        bad_call(format_args!("{operation}: {} is not a function type", function_type.describe()));
    }
}

#[track_caller]
fn check_constant_pointer<T>(operation: &str, value: Value<'_, '_, T>) {
    // SAFETY: the value is live.
    let is_pointer =
        unsafe { raw::LLVMGetTypeKind(raw::LLVMTypeOf(value.raw)) == raw::LLVMTypeKind::LLVMPointerTypeKind };
    if !value.is_constant() || !is_pointer {
        bad_call(format_args!("{operation}: {} is not a constant pointer", value.describe()));
    }
}

impl<'context, 'module> Value<'context, 'module, GlobalVariable> {
    /// This variable's next sibling in the module.
    #[must_use]
    pub fn next_global(self) -> Option<Self> {
        // SAFETY: the marker guarantees a global variable, and it stays live for `'module`.
        next_global(unsafe { raw::LLVMGetNextGlobal(self.raw) })
    }

    /// This variable's previous sibling in the module.
    #[must_use]
    pub fn previous_global(self) -> Option<Self> {
        // SAFETY: the marker guarantees a global variable, and it stays live for `'module`.
        next_global(unsafe { raw::LLVMGetPreviousGlobal(self.raw) })
    }

    /// This variable's initializer, if it has one.
    #[must_use]
    pub fn initializer(self) -> Option<Value<'context, 'module, Constant>> {
        // SAFETY: the marker guarantees a global variable.
        next_global(unsafe { raw::LLVMGetInitializer(self.raw) })
    }

    /// Sets this variable's initializer.
    ///
    /// # Panics
    /// If it isn't a constant of this variable's type, or it belongs to another module (D3).
    #[track_caller]
    pub fn set_initializer<T>(self, initializer: Value<'context, 'module, T>) {
        if !initializer.is_constant() {
            bad_call(format_args!("set_initializer: {} is not a constant", initializer.describe()));
        }
        // SAFETY: both are live for `'module`; the shim only reads them, and the type is compared before setting.
        unsafe {
            if !checks::same_identity(
                raw::LLVMTypeOf(initializer.raw) as usize,
                raw::LLVMGlobalGetValueType(self.raw) as usize,
            ) {
                bad_call(format_args!(
                    "set_initializer: {} doesn't have the type of {}",
                    initializer.describe(),
                    self.describe()
                ));
            }
            if !shim::lyftar_values_share_module(self.raw, initializer.raw) {
                bad_call(format_args!("set_initializer: {} belongs to another module", initializer.describe()));
            }
            raw::LLVMSetInitializer(self.raw, initializer.raw);
        }
    }

    /// Removes this variable's initializer, which makes it a declaration.
    pub fn remove_initializer(self) {
        // SAFETY: the marker guarantees a global variable, and a null initializer removes the old one.
        unsafe { raw::LLVMSetInitializer(self.raw, std::ptr::null_mut()) };
    }

    /// Whether this variable is thread-local.
    #[must_use]
    pub fn is_thread_local(self) -> bool {
        // SAFETY: the marker guarantees a global variable.
        unsafe { raw::LLVMIsThreadLocal(self.raw) != 0 }
    }

    /// Sets whether this variable is thread-local.
    pub fn set_thread_local(self, value: bool) {
        // SAFETY: the marker guarantees a global variable.
        unsafe { raw::LLVMSetThreadLocal(self.raw, i32::from(value)) };
    }

    /// Whether this variable can't change.
    #[must_use]
    pub fn is_global_constant(self) -> bool {
        // SAFETY: the marker guarantees a global variable.
        unsafe { raw::LLVMIsGlobalConstant(self.raw) != 0 }
    }

    /// Sets whether this variable can't change.
    pub fn set_global_constant(self, value: bool) {
        // SAFETY: the marker guarantees a global variable.
        unsafe { raw::LLVMSetGlobalConstant(self.raw, i32::from(value)) };
    }

    /// This variable's thread-local model.
    #[must_use]
    pub fn thread_local_mode(self) -> ThreadLocalMode {
        // SAFETY: the marker guarantees a global variable.
        ThreadLocalMode::from_raw(unsafe { raw::LLVMGetThreadLocalMode(self.raw) })
    }

    /// Sets this variable's thread-local model.
    pub fn set_thread_local_mode(self, mode: ThreadLocalMode) {
        // SAFETY: the marker guarantees a global variable.
        unsafe { raw::LLVMSetThreadLocalMode(self.raw, mode.to_raw()) };
    }

    /// Whether something outside this program gives this variable its first value.
    #[must_use]
    pub fn is_externally_initialized(self) -> bool {
        // SAFETY: the marker guarantees a global variable.
        unsafe { raw::LLVMIsExternallyInitialized(self.raw) != 0 }
    }

    /// Sets whether something outside this program gives this variable its first value.
    pub fn set_externally_initialized(self, value: bool) {
        // SAFETY: the marker guarantees a global variable.
        unsafe { raw::LLVMSetExternallyInitialized(self.raw, i32::from(value)) };
    }

    /// Attaches a debug-info global variable expression to this variable.
    pub fn add_debug_info(self, expression: crate::metadata::Metadata<'context, 'module>) {
        // SAFETY: the marker guarantees a global variable, and the metadata is live for `'module`.
        unsafe { raw::LLVMGlobalAddDebugInfo(self.raw, expression.raw) };
    }
}

impl<'context, 'module> Value<'context, 'module, GlobalAlias> {
    /// What this alias stands for.
    #[must_use]
    pub fn aliasee(self) -> Value<'context, 'module, Constant> {
        // SAFETY: the marker guarantees an alias, whose target is a constant.
        unsafe { Value::from_raw(raw::LLVMAliasGetAliasee(self.raw)) }
    }

    /// Sets what this alias stands for.
    ///
    /// # Panics
    /// If it isn't a constant pointer in this alias's module (D3).
    #[track_caller]
    pub fn set_aliasee<T>(self, aliasee: Value<'context, 'module, T>) {
        check_constant_pointer("set_aliasee", aliasee);
        // SAFETY: the shim only reads both; the target was just checked.
        unsafe {
            if !shim::lyftar_values_share_module(self.raw, aliasee.raw) {
                bad_call(format_args!("set_aliasee: {} belongs to another module", aliasee.describe()));
            }
            raw::LLVMAliasSetAliasee(self.raw, aliasee.raw);
        }
    }

    /// This alias's next sibling in the module.
    #[must_use]
    pub fn next_alias(self) -> Option<Self> {
        // SAFETY: the marker guarantees an alias, and it stays live for `'module`.
        next_global(unsafe { raw::LLVMGetNextGlobalAlias(self.raw) })
    }

    /// This alias's previous sibling in the module.
    #[must_use]
    pub fn previous_alias(self) -> Option<Self> {
        // SAFETY: the marker guarantees an alias, and it stays live for `'module`.
        next_global(unsafe { raw::LLVMGetPreviousGlobalAlias(self.raw) })
    }
}

impl<'context, 'module> Value<'context, 'module, GlobalIndirectFunction> {
    /// What picks the implementation.
    #[must_use]
    pub fn resolver(self) -> Value<'context, 'module, Constant> {
        // SAFETY: the marker guarantees an indirect function, whose resolver is a constant.
        unsafe { Value::from_raw(raw::LLVMGetGlobalIFuncResolver(self.raw)) }
    }

    /// Sets what picks the implementation.
    ///
    /// # Panics
    /// If it isn't a constant pointer in this one's module (D3).
    #[track_caller]
    pub fn set_resolver<T>(self, resolver: Value<'context, 'module, T>) {
        check_constant_pointer("set_resolver", resolver);
        // SAFETY: the shim only reads both; the resolver was just checked.
        unsafe {
            if !shim::lyftar_values_share_module(self.raw, resolver.raw) {
                bad_call(format_args!("set_resolver: {} belongs to another module", resolver.describe()));
            }
            raw::LLVMSetGlobalIFuncResolver(self.raw, resolver.raw);
        }
    }

    /// The next indirect function in the module.
    #[must_use]
    pub fn next_indirect_function(self) -> Option<Self> {
        // SAFETY: the marker guarantees an indirect function, and it stays live for `'module`.
        next_global(unsafe { raw::LLVMGetNextGlobalIFunc(self.raw) })
    }

    /// The previous indirect function in the module.
    #[must_use]
    pub fn previous_indirect_function(self) -> Option<Self> {
        // SAFETY: the marker guarantees an indirect function, and it stays live for `'module`.
        next_global(unsafe { raw::LLVMGetPreviousGlobalIFunc(self.raw) })
    }
}

/// The blocks of a function, from [`Value::basic_blocks`].
pub struct BasicBlocks<'context, 'module> {
    next: Option<BasicBlock<'context, 'module>>,
}

impl<'context, 'module> Iterator for BasicBlocks<'context, 'module> {
    type Item = BasicBlock<'context, 'module>;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.next?;
        self.next = current.next();
        Some(current)
    }
}

/// The instructions of a whole function, from [`Value::function_instructions`], block by block.
pub struct FunctionInstructions<'context, 'module> {
    next_block: Option<BasicBlock<'context, 'module>>,
    current: crate::basic_block::Instructions<'context, 'module>,
}

impl<'context, 'module> Iterator for FunctionInstructions<'context, 'module> {
    type Item = Value<'context, 'module, crate::values::Instruction>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(instruction) = self.current.next() {
                return Some(instruction);
            }
            let block = self.next_block?;
            self.next_block = block.next();
            self.current = block.instructions();
        }
    }
}

impl<'context, 'module, R, P> Value<'context, 'module, VariadicFunction<R, P>> {
    /// This function as a plain, dynamic one, which every function is. Nothing needs checking. It is the way to everything
    /// else a function has.
    #[must_use]
    pub fn as_function(self) -> Value<'context, 'module, Function> {
        // SAFETY: the marker guarantees a function.
        unsafe { Value::from_raw(self.raw) }
    }
}

impl<'context, 'module, R, P> Value<'context, 'module, Function<R, P>> {
    /// This function as a plain, dynamic one, which every function is. Nothing needs checking.
    #[must_use]
    pub fn as_function(self) -> Value<'context, 'module, Function> {
        // SAFETY: the marker guarantees a function.
        unsafe { Value::from_raw(self.raw) }
    }

    /// Adds a new block at the end of this function.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn append_basic_block(self, name: &str) -> BasicBlock<'context, 'module> {
        let name = c_string(name, "append_basic_block");
        // SAFETY: the function is live for `'module`, and `name` is a valid C string for the call.
        unsafe {
            let context = raw::LLVMGetValueContext(self.raw);
            BasicBlock::from_raw(raw::LLVMAppendBasicBlockInContext(context, self.raw, name.as_ptr()))
        }
    }

    /// How many blocks this function has.
    #[must_use]
    pub fn basic_block_count(self) -> u32 {
        // SAFETY: the marker guarantees a function.
        unsafe { raw::LLVMCountBasicBlocks(self.raw) }
    }

    /// This function's blocks, in order.
    #[must_use]
    pub fn basic_blocks(self) -> BasicBlocks<'context, 'module> {
        BasicBlocks { next: self.first_basic_block() }
    }

    /// All this function's instructions, block by block.
    ///
    /// The list of them is read as it goes, so instructions added meanwhile may or may not show up.
    #[must_use]
    pub fn function_instructions(self) -> FunctionInstructions<'context, 'module> {
        let mut blocks = self.basic_blocks();
        match blocks.next() {
            Some(block) => FunctionInstructions { next_block: block.next(), current: block.instructions() },
            None => FunctionInstructions { next_block: None, current: crate::basic_block::Instructions::empty() },
        }
    }

    /// This function's first block.
    #[must_use]
    pub fn first_basic_block(self) -> Option<BasicBlock<'context, 'module>> {
        // SAFETY: the marker guarantees a function.
        let first = unsafe { raw::LLVMGetFirstBasicBlock(self.raw) };
        // SAFETY: LLVM said it is a block in this function, live for `'module`.
        (!first.is_null()).then(|| unsafe { BasicBlock::from_raw(first) })
    }

    /// This function's last block.
    #[must_use]
    pub fn last_basic_block(self) -> Option<BasicBlock<'context, 'module>> {
        // SAFETY: the marker guarantees a function.
        let last = unsafe { raw::LLVMGetLastBasicBlock(self.raw) };
        // SAFETY: LLVM said it is a block in this function, live for `'module`.
        (!last.is_null()).then(|| unsafe { BasicBlock::from_raw(last) })
    }

    /// This function's entry block, if it has any blocks.
    #[must_use]
    pub fn entry_basic_block(self) -> Option<BasicBlock<'context, 'module>> {
        // A function with no blocks has no entry, and asking LLVM would hit an assertion.
        self.first_basic_block()
    }

    /// How many instructions this function has.
    #[must_use]
    pub fn instruction_count(self) -> u32 {
        // SAFETY: the marker guarantees a function.
        unsafe { shim::lyftar_function_instruction_count(self.raw) }
    }

    /// The next function in the module.
    #[must_use]
    pub fn next_function(self) -> Option<Self> {
        // SAFETY: the marker guarantees a function, which stays live for `'module`.
        next_global(unsafe { raw::LLVMGetNextFunction(self.raw) })
    }

    /// The previous function in the module.
    #[must_use]
    pub fn previous_function(self) -> Option<Self> {
        // SAFETY: the marker guarantees a function, which stays live for `'module`.
        next_global(unsafe { raw::LLVMGetPreviousFunction(self.raw) })
    }

    /// How many parameters this function has.
    #[must_use]
    pub fn parameter_count(self) -> u32 {
        // SAFETY: the marker guarantees a function.
        unsafe { raw::LLVMCountParams(self.raw) }
    }

    /// The parameter at `index`.
    ///
    /// # Panics
    /// If the function has no such parameter (D3).
    #[must_use]
    #[track_caller]
    pub fn parameter(self, index: u32) -> Value<'context, 'module> {
        let count = self.parameter_count();
        if !checks::index_in_bounds(index, count) {
            bad_call(format_args!("parameter: index {index} is out of bounds for a function of {count} parameters"));
        }
        // SAFETY: the index was just checked.
        unsafe { Value::from_raw(raw::LLVMGetParam(self.raw, index)) }
    }

    /// All the parameters.
    #[must_use]
    pub fn parameters(self) -> Vec<Value<'context, 'module, Argument>> {
        let mut parameters = vec![std::ptr::null_mut(); self.parameter_count() as usize];
        // SAFETY: the marker guarantees a function, and the buffer has room for all its parameters.
        unsafe { raw::LLVMGetParams(self.raw, parameters.as_mut_ptr()) };
        // SAFETY: LLVM just wrote live arguments there.
        parameters.into_iter().map(|parameter| unsafe { Value::from_raw(parameter) }).collect()
    }

    /// The first parameter.
    #[must_use]
    pub fn first_parameter(self) -> Option<Value<'context, 'module, Argument>> {
        if self.parameter_count() == 0 {
            return None;
        }
        // SAFETY: just checked it has one.
        next_global(unsafe { raw::LLVMGetFirstParam(self.raw) })
    }

    /// The last parameter.
    #[must_use]
    pub fn last_parameter(self) -> Option<Value<'context, 'module, Argument>> {
        if self.parameter_count() == 0 {
            return None;
        }
        // SAFETY: just checked it has one.
        next_global(unsafe { raw::LLVMGetLastParam(self.raw) })
    }

    /// This function's calling convention.
    #[must_use]
    pub fn calling_convention(self) -> CallingConvention {
        // SAFETY: the marker guarantees a function.
        CallingConvention(unsafe { raw::LLVMGetFunctionCallConv(self.raw) })
    }

    /// Sets this function's calling convention.
    ///
    /// # Panics
    /// If LLVM can't hold it, past 1023 (D3).
    #[track_caller]
    pub fn set_calling_convention(self, convention: CallingConvention) {
        let number = convention.checked_number("set_calling_convention");
        // SAFETY: the marker guarantees a function, and the number was just checked.
        unsafe { raw::LLVMSetFunctionCallConv(self.raw, number) };
    }

    /// The name of this function's garbage collector, if it has one.
    #[must_use]
    pub fn garbage_collector(self) -> Option<String> {
        // SAFETY: the marker guarantees a function, and the name is copied right away.
        let name = unsafe { raw::LLVMGetGC(self.raw) };
        (!name.is_null()).then(|| from_borrowed(name))
    }

    /// Sets the name of this function's garbage collector.
    ///
    /// # Panics
    /// If `name` has a NUL byte inside it (D3).
    #[track_caller]
    pub fn set_garbage_collector(self, name: &str) {
        let name = c_string(name, "set_garbage_collector");
        // SAFETY: the marker guarantees a function, and `name` is a valid C string for the call.
        unsafe { raw::LLVMSetGC(self.raw, name.as_ptr()) };
    }

    /// This function's personality, used for its landing pads, if it has one.
    #[must_use]
    pub fn personality(self) -> Option<Value<'context, 'module, Constant>> {
        // SAFETY: the marker guarantees a function, and LLVM says whether it has one before it is read.
        unsafe {
            (raw::LLVMHasPersonalityFn(self.raw) != 0).then(|| Value::from_raw(raw::LLVMGetPersonalityFn(self.raw)))
        }
    }

    /// Sets this function's personality.
    ///
    /// # Panics
    /// If it isn't a constant pointer in this function's module (D3).
    #[track_caller]
    pub fn set_personality<T>(self, personality: Value<'context, 'module, T>) {
        check_constant_pointer("set_personality", personality);
        // SAFETY: the shim only reads both; the personality was just checked.
        unsafe {
            if !shim::lyftar_values_share_module(self.raw, personality.raw) {
                bad_call(format_args!("set_personality: {} belongs to another module", personality.describe()));
            }
            raw::LLVMSetPersonalityFn(self.raw, personality.raw);
        }
    }

    /// The data placed before this function's code, if it has any.
    #[must_use]
    pub fn prefix_data(self) -> Option<Value<'context, 'module, Constant>> {
        // SAFETY: the marker guarantees a function, and LLVM says whether it has one before it is read.
        unsafe { (raw::LLVMHasPrefixData(self.raw) != 0).then(|| Value::from_raw(raw::LLVMGetPrefixData(self.raw))) }
    }

    /// Sets the data placed before this function's code.
    ///
    /// # Panics
    /// If it isn't a constant in this function's module (D3).
    #[track_caller]
    pub fn set_prefix_data<T>(self, data: Value<'context, 'module, T>) {
        self.check_function_data("set_prefix_data", data);
        // SAFETY: the marker guarantees a function, and the data was just checked.
        unsafe { raw::LLVMSetPrefixData(self.raw, data.raw) };
    }

    /// The data placed at the very start of this function's code, if it has any.
    #[must_use]
    pub fn prologue_data(self) -> Option<Value<'context, 'module, Constant>> {
        // SAFETY: the marker guarantees a function, and LLVM says whether it has one before it is read.
        unsafe {
            (raw::LLVMHasPrologueData(self.raw) != 0).then(|| Value::from_raw(raw::LLVMGetPrologueData(self.raw)))
        }
    }

    /// Sets the data placed at the very start of this function's code.
    ///
    /// # Panics
    /// If it isn't a constant in this function's module (D3).
    #[track_caller]
    pub fn set_prologue_data<T>(self, data: Value<'context, 'module, T>) {
        self.check_function_data("set_prologue_data", data);
        // SAFETY: the marker guarantees a function, and the data was just checked.
        unsafe { raw::LLVMSetPrologueData(self.raw, data.raw) };
    }

    #[track_caller]
    fn check_function_data<T>(self, operation: &str, data: Value<'context, 'module, T>) {
        if !data.is_constant() {
            bad_call(format_args!("{operation}: {} is not a constant", data.describe()));
        }
        // SAFETY: the shim only reads both.
        if !unsafe { shim::lyftar_values_share_module(self.raw, data.raw) } {
            bad_call(format_args!("{operation}: {} belongs to another module", data.describe()));
        }
    }

    #[track_caller]
    fn check_attribute_location(self, operation: &str, location: AttributeLocation) {
        if let AttributeLocation::Parameter(index) = location {
            let count = self.parameter_count();
            if !checks::index_in_bounds(index, count) {
                bad_call(format_args!("{operation}: parameter {index} is out of bounds for {count} parameters"));
            }
        }
    }

    /// Adds an attribute at a location of this function.
    ///
    /// # Panics
    /// If the location is a parameter this function doesn't have (D3).
    #[track_caller]
    pub fn add_attribute(self, location: AttributeLocation, attribute: Attribute<'context>) {
        self.check_attribute_location("add_attribute", location);
        // SAFETY: the location was just checked, and the attribute is live for `'context`.
        unsafe { raw::LLVMAddAttributeAtIndex(self.raw, location.to_raw(), attribute.raw) };
    }

    /// The attributes at a location of this function.
    ///
    /// # Panics
    /// If the location is a parameter this function doesn't have (D3).
    #[must_use]
    #[track_caller]
    pub fn attributes(self, location: AttributeLocation) -> Vec<Attribute<'context>> {
        self.check_attribute_location("attributes", location);
        // SAFETY: the location was just checked, and the buffer has room for all of them.
        unsafe {
            let count = raw::LLVMGetAttributeCountAtIndex(self.raw, location.to_raw());
            let mut attributes = vec![std::ptr::null_mut(); count as usize];
            raw::LLVMGetAttributesAtIndex(self.raw, location.to_raw(), attributes.as_mut_ptr());
            attributes.into_iter().map(|attribute| Attribute::from_raw(attribute)).collect()
        }
    }

    /// The attribute of an enum kind at a location of this function, if it has it.
    ///
    /// # Panics
    /// If the location is a parameter this function doesn't have (D3).
    #[must_use]
    #[track_caller]
    pub fn enum_attribute(self, location: AttributeLocation, kind: AttributeKind) -> Option<Attribute<'context>> {
        self.check_attribute_location("enum_attribute", location);
        // SAFETY: the location was just checked.
        let found = unsafe { raw::LLVMGetEnumAttributeAtIndex(self.raw, location.to_raw(), kind.0) };
        // SAFETY: LLVM gave back a live attribute.
        (!found.is_null()).then(|| unsafe { Attribute::from_raw(found) })
    }

    /// The attribute with a string key at a location of this function, if it has it.
    ///
    /// # Panics
    /// If the location is a parameter this function doesn't have (D3).
    #[must_use]
    #[track_caller]
    pub fn string_attribute(self, location: AttributeLocation, key: &str) -> Option<Attribute<'context>> {
        self.check_attribute_location("string_attribute", location);
        let length = length_as_u32(key.len(), "string_attribute");
        // SAFETY: the location was just checked; LLVM reads `length` bytes of the key.
        let found =
            unsafe { raw::LLVMGetStringAttributeAtIndex(self.raw, location.to_raw(), key.as_ptr().cast(), length) };
        // SAFETY: LLVM gave back a live attribute.
        (!found.is_null()).then(|| unsafe { Attribute::from_raw(found) })
    }

    /// Removes the attribute of an enum kind at a location of this function.
    ///
    /// # Panics
    /// If the location is a parameter this function doesn't have (D3).
    #[track_caller]
    pub fn remove_enum_attribute(self, location: AttributeLocation, kind: AttributeKind) {
        self.check_attribute_location("remove_enum_attribute", location);
        // SAFETY: the location was just checked.
        unsafe { raw::LLVMRemoveEnumAttributeAtIndex(self.raw, location.to_raw(), kind.0) };
    }

    /// Removes the attribute with a string key at a location of this function.
    ///
    /// # Panics
    /// If the location is a parameter this function doesn't have (D3).
    #[track_caller]
    pub fn remove_string_attribute(self, location: AttributeLocation, key: &str) {
        self.check_attribute_location("remove_string_attribute", location);
        let length = length_as_u32(key.len(), "remove_string_attribute");
        // SAFETY: the location was just checked; LLVM reads `length` bytes of the key.
        unsafe { raw::LLVMRemoveStringAttributeAtIndex(self.raw, location.to_raw(), key.as_ptr().cast(), length) };
    }

    /// Adds a string attribute to the function as a whole, from its key and value.
    ///
    /// # Panics
    /// If either has a NUL byte inside it (D3).
    #[track_caller]
    pub fn add_target_dependent_attribute(self, key: &str, value: &str) {
        let key = c_string(key, "add_target_dependent_attribute");
        let value = c_string(value, "add_target_dependent_attribute");
        // SAFETY: both are valid C strings for the call.
        unsafe { raw::LLVMAddTargetDependentFunctionAttr(self.raw, key.as_ptr(), value.as_ptr()) };
    }

    /// The intrinsic this function is, if it is one.
    #[must_use]
    pub fn intrinsic(self) -> Option<crate::intrinsics::Intrinsic> {
        // SAFETY: the marker guarantees a function.
        let id = unsafe { raw::LLVMGetIntrinsicID(self.raw) };
        (id != 0).then_some(crate::intrinsics::Intrinsic(id))
    }

    /// This function's debug-info subprogram, if it has one.
    #[must_use]
    pub fn subprogram(self) -> Option<crate::metadata::Metadata<'context, 'module>> {
        // SAFETY: the marker guarantees a function.
        let found = unsafe { raw::LLVMGetSubprogram(self.raw) };
        // SAFETY: LLVM gave back live metadata.
        (!found.is_null()).then(|| unsafe { crate::metadata::Metadata::from_raw(found) })
    }

    /// Sets this function's debug-info subprogram.
    pub fn set_subprogram(self, subprogram: crate::metadata::Metadata<'context, 'module>) {
        // SAFETY: the marker guarantees a function, and the metadata is live for `'module`.
        unsafe { raw::LLVMSetSubprogram(self.raw, subprogram.raw) };
    }

    /// Runs LLVM's verifier on this function alone, and says whether it is valid.
    #[must_use]
    pub fn verify(self) -> bool {
        // SAFETY: the marker guarantees a function; this action only returns a status.
        unsafe { raw::LLVMVerifyFunction(self.raw, raw::LLVMVerifierFailureAction::LLVMReturnStatusAction) == 0 }
    }
}

impl<'context, 'module> Value<'context, 'module, Argument> {
    /// The function this argument belongs to.
    #[must_use]
    pub fn parent_function(self) -> Value<'context, 'module, Function> {
        // SAFETY: the marker guarantees an argument, and Lyftar's are all in a function.
        unsafe { Value::from_raw(raw::LLVMGetParamParent(self.raw)) }
    }

    /// The next parameter of the function.
    #[must_use]
    pub fn next_parameter(self) -> Option<Self> {
        // SAFETY: the marker guarantees an argument.
        next_global(unsafe { raw::LLVMGetNextParam(self.raw) })
    }

    /// The previous parameter of the function.
    #[must_use]
    pub fn previous_parameter(self) -> Option<Self> {
        // SAFETY: the marker guarantees an argument.
        next_global(unsafe { raw::LLVMGetPreviousParam(self.raw) })
    }

    /// Sets the alignment this parameter promises.
    ///
    /// # Panics
    /// If it isn't a power of two (D3).
    #[track_caller]
    pub fn set_alignment_in_bytes(self, alignment_in_bytes: u32) {
        if !checks::is_power_of_two(alignment_in_bytes) {
            bad_call(format_args!("set_alignment_in_bytes: {alignment_in_bytes} is not a power of two"));
        }
        // SAFETY: the marker guarantees an argument, and the alignment was just checked.
        unsafe { raw::LLVMSetParamAlignment(self.raw, alignment_in_bytes) };
    }
}
