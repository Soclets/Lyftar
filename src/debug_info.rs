//! Building debug information.
//!
//! LLVM's debug info C API takes all its pieces as one untyped metadata handle and casts them without checking in a release
//! build, so each call here checks each piece is the class LLVM wants, and panics otherwise (D3).

// LLVM's own functions take this many arguments, and each one here is spelled out by name.
#![allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]

use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::block::Block;
use crate::checks;
use crate::enums::llvm_enum;
use crate::metadata::Metadata;
use crate::module::Module;
use crate::raw;
use crate::shim;
use crate::text::{from_owned_message, from_pointer_and_length};
use crate::types::{Invariant, length_as_u32};
use crate::values::{Function, GlobalVariable, InstructionMarker, Value};

/// The classes LLVM's debug info calls want, numbered as in the shim's `lyftar_metadata_is`.
#[derive(Clone, Copy)]
enum Class {
    File = 0,
    Scope = 1,
    Type = 2,
    LocalVariable = 3,
    Expression = 4,
    Location = 5,
    Subprogram = 6,
    Label = 7,
    Node = 8,
    GlobalVariableExpression = 9,
    Variable = 10,
    MacroFile = 11,
    DebugInfoNode = 13,
    LocalScope = 14,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::File => "file",
            Class::Scope => "scope",
            Class::Type => "type",
            Class::LocalVariable => "local variable",
            Class::Expression => "expression",
            Class::Location => "location",
            Class::Subprogram => "subprogram",
            Class::Label => "label",
            Class::Node => "metadata node",
            Class::GlobalVariableExpression => "global variable expression",
            Class::Variable => "variable",
            Class::MacroFile => "macro file",
            Class::DebugInfoNode => "debug info node",
            Class::LocalScope => "local scope",
        }
    }
}

// trace: D3
#[track_caller]
fn check(operation: &str, metadata: Metadata<'_, '_>, class: Class) {
    // SAFETY: the metadata is live; the shim only reads it.
    if !unsafe { shim::lyftar_metadata_is(metadata.raw, class as u32) } {
        bad_call(format_args!("{operation}: that metadata is not a {}", class.name()));
    }
}

#[track_caller]
fn check_optional(operation: &str, metadata: Option<Metadata<'_, '_>>, class: Class) -> raw::LLVMMetadataRef {
    match metadata {
        Some(found) => {
            check(operation, found, class);
            found.raw
        }
        None => std::ptr::null_mut(),
    }
}

#[track_caller]
fn check_list(operation: &str, metadata: &[Metadata<'_, '_>], class: Class) {
    for item in metadata {
        check(operation, *item, class);
    }
}

llvm_enum! {
    /// How much debug information a compile unit gets.
    pub enum DwarfEmissionKind: LLVMDWARFEmissionKind {
        /// None.
        None = LLVMDWARFEmissionNone,
        /// Everything.
        Full = LLVMDWARFEmissionFull,
        /// Only what maps code to lines.
        LineTablesOnly = LLVMDWARFEmissionLineTablesOnly,
    }
}

llvm_enum! {
    /// The source language of a compile unit.
    pub enum DwarfSourceLanguage: LLVMDWARFSourceLanguage {
        /// C89.
        C89 = LLVMDWARFSourceLanguageC89,
        /// C.
        C = LLVMDWARFSourceLanguageC,
        /// Ada 83.
        Ada83 = LLVMDWARFSourceLanguageAda83,
        /// C++.
        CPlusPlus = LLVMDWARFSourceLanguageC_plus_plus,
        /// Cobol 74.
        Cobol74 = LLVMDWARFSourceLanguageCobol74,
        /// Cobol 85.
        Cobol85 = LLVMDWARFSourceLanguageCobol85,
        /// Fortran 77.
        Fortran77 = LLVMDWARFSourceLanguageFortran77,
        /// Fortran 90.
        Fortran90 = LLVMDWARFSourceLanguageFortran90,
        /// Pascal 83.
        Pascal83 = LLVMDWARFSourceLanguagePascal83,
        /// Modula-2.
        Modula2 = LLVMDWARFSourceLanguageModula2,
        /// Java.
        Java = LLVMDWARFSourceLanguageJava,
        /// C99.
        C99 = LLVMDWARFSourceLanguageC99,
        /// Ada 95.
        Ada95 = LLVMDWARFSourceLanguageAda95,
        /// Fortran 95.
        Fortran95 = LLVMDWARFSourceLanguageFortran95,
        /// PL/I.
        Pli = LLVMDWARFSourceLanguagePLI,
        /// Objective-C.
        ObjectiveC = LLVMDWARFSourceLanguageObjC,
        /// Objective-C++.
        ObjectiveCPlusPlus = LLVMDWARFSourceLanguageObjC_plus_plus,
        /// UPC.
        Upc = LLVMDWARFSourceLanguageUPC,
        /// D.
        D = LLVMDWARFSourceLanguageD,
        /// Python.
        Python = LLVMDWARFSourceLanguagePython,
        /// OpenCL.
        OpenCl = LLVMDWARFSourceLanguageOpenCL,
        /// Go.
        Go = LLVMDWARFSourceLanguageGo,
        /// Modula-3.
        Modula3 = LLVMDWARFSourceLanguageModula3,
        /// Haskell.
        Haskell = LLVMDWARFSourceLanguageHaskell,
        /// C++03.
        CPlusPlus03 = LLVMDWARFSourceLanguageC_plus_plus_03,
        /// C++11.
        CPlusPlus11 = LLVMDWARFSourceLanguageC_plus_plus_11,
        /// OCaml.
        OCaml = LLVMDWARFSourceLanguageOCaml,
        /// Rust.
        Rust = LLVMDWARFSourceLanguageRust,
        /// C11.
        C11 = LLVMDWARFSourceLanguageC11,
        /// Swift.
        Swift = LLVMDWARFSourceLanguageSwift,
        /// Julia.
        Julia = LLVMDWARFSourceLanguageJulia,
        /// Dylan.
        Dylan = LLVMDWARFSourceLanguageDylan,
        /// C++14.
        CPlusPlus14 = LLVMDWARFSourceLanguageC_plus_plus_14,
        /// Fortran 2003.
        Fortran03 = LLVMDWARFSourceLanguageFortran03,
        /// Fortran 2008.
        Fortran08 = LLVMDWARFSourceLanguageFortran08,
        /// RenderScript.
        RenderScript = LLVMDWARFSourceLanguageRenderScript,
        /// BLISS.
        Bliss = LLVMDWARFSourceLanguageBLISS,
        /// Kotlin.
        Kotlin = LLVMDWARFSourceLanguageKotlin,
        /// Zig.
        Zig = LLVMDWARFSourceLanguageZig,
        /// Crystal.
        Crystal = LLVMDWARFSourceLanguageCrystal,
        /// C++17.
        CPlusPlus17 = LLVMDWARFSourceLanguageC_plus_plus_17,
        /// C++20.
        CPlusPlus20 = LLVMDWARFSourceLanguageC_plus_plus_20,
        /// C17.
        C17 = LLVMDWARFSourceLanguageC17,
        /// Fortran 2018.
        Fortran18 = LLVMDWARFSourceLanguageFortran18,
        /// Ada 2005.
        Ada2005 = LLVMDWARFSourceLanguageAda2005,
        /// Ada 2012.
        Ada2012 = LLVMDWARFSourceLanguageAda2012,
        /// HIP.
        Hip = LLVMDWARFSourceLanguageHIP,
        /// Assembly.
        Assembly = LLVMDWARFSourceLanguageAssembly,
        /// C#.
        CSharp = LLVMDWARFSourceLanguageC_sharp,
        /// Mojo.
        Mojo = LLVMDWARFSourceLanguageMojo,
        /// GLSL.
        Glsl = LLVMDWARFSourceLanguageGLSL,
        /// GLSL ES.
        GlslEs = LLVMDWARFSourceLanguageGLSL_ES,
        /// HLSL.
        Hlsl = LLVMDWARFSourceLanguageHLSL,
        /// OpenCL C++.
        OpenClCPlusPlus = LLVMDWARFSourceLanguageOpenCL_CPP,
        /// C++ for OpenCL.
        CPlusPlusForOpenCl = LLVMDWARFSourceLanguageCPP_for_OpenCL,
        /// SYCL.
        Sycl = LLVMDWARFSourceLanguageSYCL,
        /// Ruby.
        Ruby = LLVMDWARFSourceLanguageRuby,
        /// Move.
        Move = LLVMDWARFSourceLanguageMove,
        /// Hylo.
        Hylo = LLVMDWARFSourceLanguageHylo,
        /// Metal.
        Metal = LLVMDWARFSourceLanguageMetal,
        /// MIPS assembler.
        MipsAssembler = LLVMDWARFSourceLanguageMips_Assembler,
        /// Google's RenderScript.
        GoogleRenderScript = LLVMDWARFSourceLanguageGOOGLE_RenderScript,
        /// Borland Delphi.
        BorlandDelphi = LLVMDWARFSourceLanguageBORLAND_Delphi,
    }
}

llvm_enum! {
    /// What a file's checksum is computed with.
    pub enum ChecksumKind: LLVMChecksumKind {
        /// MD5.
        Md5 = CSK_MD5,
        /// SHA-1.
        Sha1 = CSK_SHA1,
        /// SHA-256.
        Sha256 = CSK_SHA256,
    }
}

llvm_enum! {
    /// What a macro entry records.
    pub enum DwarfMacroRecordType: LLVMDWARFMacinfoRecordType {
        /// A definition.
        Define = LLVMDWARFMacinfoRecordTypeDefine,
        /// A removal of a definition.
        Undefine = LLVMDWARFMacinfoRecordTypeMacro,
        /// The start of a file.
        StartFile = LLVMDWARFMacinfoRecordTypeStartFile,
        /// The end of a file.
        EndFile = LLVMDWARFMacinfoRecordTypeEndFile,
        /// A vendor's own.
        VendorExtension = LLVMDWARFMacinfoRecordTypeVendorExt,
    }
}

llvm_enum! {
    /// What kind of debug record a record is.
    pub enum DebugRecordKind: LLVMDbgRecordKind {
        /// A label.
        Label = LLVMDbgRecordLabel,
        /// A variable's declaration.
        Declare = LLVMDbgRecordDeclare,
        /// A variable's value.
        Value = LLVMDbgRecordValue,
        /// An assignment to a variable.
        Assign = LLVMDbgRecordAssign,
    }
}

/// Debug information flags, which can be combined with `|`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DebugInfoFlags(pub u32);

impl DebugInfoFlags {
    /// No flags.
    pub const ZERO: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagZero.0);
    /// Private.
    pub const PRIVATE: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagPrivate.0);
    /// Protected.
    pub const PROTECTED: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagProtected.0);
    /// Public.
    pub const PUBLIC: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagPublic.0);
    /// Only declared.
    pub const FORWARD_DECLARATION: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagFwdDecl.0);
    /// Virtual.
    pub const VIRTUAL: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagVirtual.0);
    /// Made by the compiler.
    pub const ARTIFICIAL: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagArtificial.0);
    /// Explicit.
    pub const EXPLICIT: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagExplicit.0);
    /// Prototyped.
    pub const PROTOTYPED: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagPrototyped.0);
    /// `this`, or the like.
    pub const OBJECT_POINTER: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagObjectPointer.0);
    /// A vector.
    pub const VECTOR: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagVector.0);
    /// A static member.
    pub const STATIC_MEMBER: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagStaticMember.0);
    /// A bit field.
    pub const BIT_FIELD: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagBitField.0);
    /// Never returns.
    pub const NO_RETURN: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagNoReturn.0);
    /// Passed by value.
    pub const TYPE_PASS_BY_VALUE: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagTypePassByValue.0);
    /// Passed by reference.
    pub const TYPE_PASS_BY_REFERENCE: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagTypePassByReference.0);
    /// A thunk.
    pub const THUNK: Self = DebugInfoFlags(raw::LLVMDIFlags::LLVMDIFlagThunk.0);

    fn to_raw(self) -> raw::LLVMDIFlags {
        raw::LLVMDIFlags(self.0)
    }
}

impl std::ops::BitOr for DebugInfoFlags {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        DebugInfoFlags(self.0 | other.0)
    }
}

/// A temporary metadata node, standing in for one that isn't made yet. Dropping it makes what uses it null.
pub struct TemporaryMetadata<'context, 'module> {
    raw: raw::LLVMMetadataRef,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl Drop for TemporaryMetadata<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the temporary node, and nothing else can free it.
        unsafe { shim::lyftar_temporary_metadata_dispose(self.raw) };
    }
}

impl<'context, 'module> TemporaryMetadata<'context, 'module> {
    /// A new temporary node with these operands.
    #[must_use]
    pub fn new(context: crate::Context<'context>, operands: &[Metadata<'context, 'module>]) -> Self {
        // SAFETY: LLVM only reads the operands, and `Metadata` has the layout of `LLVMMetadataRef`; the wrapper owns the node.
        let raw = unsafe { raw::LLVMTemporaryMDNode(context.raw, operands.as_ptr().cast_mut().cast(), operands.len()) };
        TemporaryMetadata { raw, marker: PhantomData }
    }

    /// This node, to use where the real one will go. The result can't outlive it.
    #[must_use]
    pub fn as_metadata(&self) -> Metadata<'context, '_> {
        // SAFETY: the node lives as long as this borrow.
        unsafe { Metadata::from_raw(self.raw) }
    }

    /// Replaces all uses of this node with the real one, and frees it.
    pub fn replace_all_uses_with(self, replacement: Metadata<'context, 'module>) {
        let raw = self.raw;
        std::mem::forget(self);
        // SAFETY: LLVM takes the temporary node here, which this consumed, and both are live.
        unsafe { raw::LLVMMetadataReplaceAllUsesWith(raw, replacement.raw) };
    }
}

/// A record of debug information beside an instruction.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DebugRecord<'context, 'module> {
    raw: raw::LLVMDbgRecordRef,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl<'context, 'module> DebugRecord<'context, 'module> {
    fn optional(raw: raw::LLVMDbgRecordRef) -> Option<Self> {
        (!raw.is_null()).then_some(DebugRecord { raw, marker: PhantomData })
    }

    /// LLVM's text for this record. Until its block has a terminator, LLVM can't print it, and this says so.
    #[must_use]
    pub fn print_to_string(self) -> String {
        // SAFETY: the record is live for `'module`, and only a record with an instruction after it goes to LLVM, which
        // allocates the text for the caller to free.
        unsafe {
            if shim::lyftar_debug_record_is_trailing(self.raw) {
                return String::from("<a debug record in a block with no terminator yet>");
            }
            from_owned_message(raw::LLVMPrintDbgRecordToString(self.raw))
        }
    }

    /// The next record beside the same instruction.
    #[must_use]
    pub fn next(self) -> Option<Self> {
        // SAFETY: the record is live for `'module`.
        Self::optional(unsafe { raw::LLVMGetNextDbgRecord(self.raw) })
    }

    /// The previous record beside the same instruction.
    #[must_use]
    pub fn previous(self) -> Option<Self> {
        // SAFETY: the record is live for `'module`.
        Self::optional(unsafe { raw::LLVMGetPreviousDbgRecord(self.raw) })
    }

    /// This record's debug location.
    #[must_use]
    pub fn debug_location(self) -> Metadata<'context, 'module> {
        // SAFETY: the record is live for `'module`, as is its location.
        unsafe { Metadata::from_raw(raw::LLVMDbgRecordGetDebugLoc(self.raw)) }
    }

    /// What kind of record this is.
    #[must_use]
    pub fn kind(self) -> DebugRecordKind {
        // SAFETY: the record is live for `'module`.
        DebugRecordKind::from_raw(unsafe { raw::LLVMDbgRecordGetKind(self.raw) })
    }

    #[track_caller]
    fn check_variable(self, operation: &str) {
        if self.kind() == DebugRecordKind::Label {
            bad_call(format_args!("{operation}: a label record has no variable"));
        }
    }

    /// The value this variable record is about, at an operand.
    ///
    /// # Panics
    /// If this is a label record (D3).
    #[must_use]
    #[track_caller]
    pub fn value(self, operand_index: u32) -> Value<'context, 'module> {
        self.check_variable("value");
        // SAFETY: just checked; the record is live for `'module`, as is its value.
        unsafe { Value::from_raw(raw::LLVMDbgVariableRecordGetValue(self.raw, operand_index)) }
    }

    /// The variable this variable record is about.
    ///
    /// # Panics
    /// If this is a label record (D3).
    #[must_use]
    #[track_caller]
    pub fn variable(self) -> Metadata<'context, 'module> {
        self.check_variable("variable");
        // SAFETY: just checked; the record is live for `'module`, as is its variable.
        unsafe { Metadata::from_raw(raw::LLVMDbgVariableRecordGetVariable(self.raw)) }
    }

    /// The expression of this variable record.
    ///
    /// # Panics
    /// If this is a label record (D3).
    #[must_use]
    #[track_caller]
    pub fn expression(self) -> Metadata<'context, 'module> {
        self.check_variable("expression");
        // SAFETY: just checked; the record is live for `'module`, as is its expression.
        unsafe { Metadata::from_raw(raw::LLVMDbgVariableRecordGetExpression(self.raw)) }
    }
}

impl<'context, 'module, T: InstructionMarker> Value<'context, 'module, T> {
    /// The first debug record beside this instruction.
    #[must_use]
    pub fn first_debug_record(self) -> Option<DebugRecord<'context, 'module>> {
        // SAFETY: the marker guarantees an instruction.
        DebugRecord::optional(unsafe { raw::LLVMGetFirstDbgRecord(self.raw) })
    }

    /// The last debug record beside this instruction.
    #[must_use]
    pub fn last_debug_record(self) -> Option<DebugRecord<'context, 'module>> {
        // SAFETY: the marker guarantees an instruction.
        DebugRecord::optional(unsafe { raw::LLVMGetLastDbgRecord(self.raw) })
    }
}

impl Module<'_> {
    /// Whether this module uses debug records, rather than debug intrinsics.
    #[must_use]
    pub fn uses_debug_records(&self) -> bool {
        // SAFETY: the module is live.
        unsafe { raw::LLVMIsNewDbgInfoFormat(self.raw) != 0 }
    }
}

impl<'context, 'module> Metadata<'context, 'module> {
    /// What this metadata's DWARF tag is, if it is a debug info node.
    #[must_use]
    pub fn dwarf_tag(self) -> Option<u16> {
        // SAFETY: the shim only reads it, and the tag is asked of a debug info node.
        unsafe {
            shim::lyftar_metadata_is(self.raw, Class::DebugInfoNode as u32).then(|| raw::LLVMGetDINodeTag(self.raw))
        }
    }

    /// The line of this location.
    ///
    /// # Panics
    /// If this isn't a location (D3).
    #[must_use]
    #[track_caller]
    pub fn location_line(self) -> u32 {
        check("location_line", self, Class::Location);
        // SAFETY: just checked.
        unsafe { raw::LLVMDILocationGetLine(self.raw) }
    }

    /// The column of this location.
    ///
    /// # Panics
    /// If this isn't a location (D3).
    #[must_use]
    #[track_caller]
    pub fn location_column(self) -> u32 {
        check("location_column", self, Class::Location);
        // SAFETY: just checked.
        unsafe { raw::LLVMDILocationGetColumn(self.raw) }
    }

    /// The scope of this location.
    ///
    /// # Panics
    /// If this isn't a location (D3).
    #[must_use]
    #[track_caller]
    pub fn location_scope(self) -> Metadata<'context, 'module> {
        check("location_scope", self, Class::Location);
        // SAFETY: just checked, and it lives as long as this metadata.
        unsafe { Metadata::from_raw(raw::LLVMDILocationGetScope(self.raw)) }
    }

    /// Where this location was inlined, if it was.
    ///
    /// # Panics
    /// If this isn't a location (D3).
    #[must_use]
    #[track_caller]
    pub fn location_inlined_at(self) -> Option<Metadata<'context, 'module>> {
        check("location_inlined_at", self, Class::Location);
        // SAFETY: just checked.
        let found = unsafe { raw::LLVMDILocationGetInlinedAt(self.raw) };
        // SAFETY: LLVM gave back live metadata.
        (!found.is_null()).then(|| unsafe { Metadata::from_raw(found) })
    }

    /// The file of this scope.
    ///
    /// # Panics
    /// If this isn't a scope (D3).
    #[must_use]
    #[track_caller]
    pub fn scope_file(self) -> Option<Metadata<'context, 'module>> {
        check("scope_file", self, Class::Scope);
        // SAFETY: just checked.
        let found = unsafe { raw::LLVMDIScopeGetFile(self.raw) };
        // SAFETY: LLVM gave back live metadata.
        (!found.is_null()).then(|| unsafe { Metadata::from_raw(found) })
    }

    /// This file's directory.
    ///
    /// # Panics
    /// If this isn't a file (D3).
    #[must_use]
    #[track_caller]
    pub fn file_directory(self) -> String {
        check("file_directory", self, Class::File);
        let mut length = 0;
        // SAFETY: just checked; the text is copied right away.
        let text = unsafe { raw::LLVMDIFileGetDirectory(self.raw, &raw mut length) };
        from_pointer_and_length(text, length as usize)
    }

    /// This file's name.
    ///
    /// # Panics
    /// If this isn't a file (D3).
    #[must_use]
    #[track_caller]
    pub fn file_name(self) -> String {
        check("file_name", self, Class::File);
        let mut length = 0;
        // SAFETY: just checked; the text is copied right away.
        let text = unsafe { raw::LLVMDIFileGetFilename(self.raw, &raw mut length) };
        from_pointer_and_length(text, length as usize)
    }

    /// This file's source text, if it has any.
    ///
    /// # Panics
    /// If this isn't a file (D3).
    #[must_use]
    #[track_caller]
    pub fn file_source(self) -> String {
        check("file_source", self, Class::File);
        let mut length = 0;
        // SAFETY: just checked; the text is copied right away.
        let text = unsafe { raw::LLVMDIFileGetSource(self.raw, &raw mut length) };
        from_pointer_and_length(text, length as usize)
    }

    /// This type's name.
    ///
    /// # Panics
    /// If this isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn type_name(self) -> String {
        check("type_name", self, Class::Type);
        let mut length = 0;
        // SAFETY: just checked; the text is copied right away.
        let text = unsafe { raw::LLVMDITypeGetName(self.raw, &raw mut length) };
        from_pointer_and_length(text, length)
    }

    /// This type's size.
    ///
    /// # Panics
    /// If this isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn type_size_in_bits(self) -> u64 {
        check("type_size_in_bits", self, Class::Type);
        // SAFETY: just checked.
        unsafe { raw::LLVMDITypeGetSizeInBits(self.raw) }
    }

    /// This type's offset.
    ///
    /// # Panics
    /// If this isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn type_offset_in_bits(self) -> u64 {
        check("type_offset_in_bits", self, Class::Type);
        // SAFETY: just checked.
        unsafe { raw::LLVMDITypeGetOffsetInBits(self.raw) }
    }

    /// This type's alignment.
    ///
    /// # Panics
    /// If this isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn type_alignment_in_bits(self) -> u32 {
        check("type_alignment_in_bits", self, Class::Type);
        // SAFETY: just checked.
        unsafe { raw::LLVMDITypeGetAlignInBits(self.raw) }
    }

    /// The line this type is declared on.
    ///
    /// # Panics
    /// If this isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn type_line(self) -> u32 {
        check("type_line", self, Class::Type);
        // SAFETY: just checked.
        unsafe { raw::LLVMDITypeGetLine(self.raw) }
    }

    /// This type's flags.
    ///
    /// # Panics
    /// If this isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn type_flags(self) -> DebugInfoFlags {
        check("type_flags", self, Class::Type);
        // SAFETY: just checked.
        DebugInfoFlags(unsafe { raw::LLVMDITypeGetFlags(self.raw) }.0)
    }

    /// The variable of this global variable expression.
    ///
    /// # Panics
    /// If this isn't one (D3).
    #[must_use]
    #[track_caller]
    pub fn global_variable_expression_variable(self) -> Metadata<'context, 'module> {
        check("global_variable_expression_variable", self, Class::GlobalVariableExpression);
        // SAFETY: just checked, and it lives as long as this metadata.
        unsafe { Metadata::from_raw(raw::LLVMDIGlobalVariableExpressionGetVariable(self.raw)) }
    }

    /// The expression of this global variable expression.
    ///
    /// # Panics
    /// If this isn't one (D3).
    #[must_use]
    #[track_caller]
    pub fn global_variable_expression_expression(self) -> Metadata<'context, 'module> {
        check("global_variable_expression_expression", self, Class::GlobalVariableExpression);
        // SAFETY: just checked, and it lives as long as this metadata.
        unsafe { Metadata::from_raw(raw::LLVMDIGlobalVariableExpressionGetExpression(self.raw)) }
    }

    /// This variable's file.
    ///
    /// # Panics
    /// If this isn't a variable (D3).
    #[must_use]
    #[track_caller]
    pub fn variable_file(self) -> Option<Metadata<'context, 'module>> {
        check("variable_file", self, Class::Variable);
        // SAFETY: just checked.
        let found = unsafe { raw::LLVMDIVariableGetFile(self.raw) };
        // SAFETY: LLVM gave back live metadata.
        (!found.is_null()).then(|| unsafe { Metadata::from_raw(found) })
    }

    /// This variable's scope.
    ///
    /// # Panics
    /// If this isn't a variable (D3).
    #[must_use]
    #[track_caller]
    pub fn variable_scope(self) -> Option<Metadata<'context, 'module>> {
        check("variable_scope", self, Class::Variable);
        // SAFETY: just checked.
        let found = unsafe { raw::LLVMDIVariableGetScope(self.raw) };
        // SAFETY: LLVM gave back live metadata.
        (!found.is_null()).then(|| unsafe { Metadata::from_raw(found) })
    }

    /// The line this variable is on.
    ///
    /// # Panics
    /// If this isn't a variable (D3).
    #[must_use]
    #[track_caller]
    pub fn variable_line(self) -> u32 {
        check("variable_line", self, Class::Variable);
        // SAFETY: just checked.
        unsafe { raw::LLVMDIVariableGetLine(self.raw) }
    }

    /// The line this subprogram is on.
    ///
    /// # Panics
    /// If this isn't a subprogram (D3).
    #[must_use]
    #[track_caller]
    pub fn subprogram_line(self) -> u32 {
        check("subprogram_line", self, Class::Subprogram);
        // SAFETY: just checked.
        unsafe { raw::LLVMDISubprogramGetLine(self.raw) }
    }

    /// Sets this subprogram's type.
    ///
    /// # Panics
    /// If this isn't a subprogram, or that isn't a type (D3).
    #[track_caller]
    pub fn subprogram_replace_type(self, subroutine_type: Metadata<'context, 'module>) {
        check("subprogram_replace_type", self, Class::Subprogram);
        check("subprogram_replace_type", subroutine_type, Class::Type);
        // SAFETY: just checked.
        unsafe { raw::LLVMDISubprogramReplaceType(self.raw, subroutine_type.raw) };
    }
}

/// Builds debug information in a module. [`DebugInfoBuilder::finalize`] must run before the module is verified or emitted.
pub struct DebugInfoBuilder<'context, 'module> {
    raw: raw::LLVMDIBuilderRef,
    context: raw::LLVMContextRef,
    marker: PhantomData<(Invariant<'context>, &'module ())>,
}

impl Drop for DebugInfoBuilder<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the builder.
        unsafe { raw::LLVMDisposeDIBuilder(self.raw) };
    }
}

impl<'context, 'module> DebugInfoBuilder<'context, 'module> {
    /// A builder for this module. With `allow_unresolved`, [`DebugInfoBuilder::finalize`] resolves any cycles left through
    /// temporary nodes.
    #[must_use]
    pub fn new(module: &'module Module<'context>, allow_unresolved: bool) -> Self {
        // SAFETY: the module is live for `'module`, and the wrapper owns what comes back.
        let raw_builder = unsafe {
            if allow_unresolved {
                raw::LLVMCreateDIBuilder(module.raw)
            } else {
                raw::LLVMCreateDIBuilderDisallowUnresolved(module.raw)
            }
        };
        // SAFETY: the module is live.
        let context = unsafe { raw::LLVMGetModuleContext(module.raw) };
        DebugInfoBuilder { raw: raw_builder, context, marker: PhantomData }
    }

    /// The raw builder.
    ///
    /// # Safety
    /// Raw calls can break what the safe API promises (D15).
    #[must_use]
    pub unsafe fn as_raw(&self) -> raw::LLVMDIBuilderRef {
        self.raw
    }

    /// Resolves everything built so far.
    pub fn finalize(&self) {
        // SAFETY: the builder is live.
        unsafe { raw::LLVMDIBuilderFinalize(self.raw) };
    }

    /// Resolves one subprogram's variables and labels ahead of the rest.
    ///
    /// # Panics
    /// If it isn't a subprogram (D3).
    #[track_caller]
    pub fn finalize_subprogram(&self, subprogram: Metadata<'context, 'module>) {
        check("finalize_subprogram", subprogram, Class::Subprogram);
        // SAFETY: just checked.
        unsafe { raw::LLVMDIBuilderFinalizeSubprogram(self.raw, subprogram.raw) };
    }

    /// A compile unit.
    ///
    /// # Panics
    /// If `file` isn't a file (D3).
    #[must_use]
    #[track_caller]
    pub fn create_compile_unit(
        &self,
        language: DwarfSourceLanguage,
        file: Metadata<'context, 'module>,
        producer: &str,
        is_optimized: bool,
        flags: &str,
        runtime_version: u32,
        split_name: &str,
        kind: DwarfEmissionKind,
        dwarf_object_identifier: u32,
        split_debug_inlining: bool,
        debug_info_for_profiling: bool,
        system_root: &str,
        software_development_kit: &str,
    ) -> Metadata<'context, 'module> {
        check("create_compile_unit", file, Class::File);
        // SAFETY: just checked; LLVM reads the lengths given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateCompileUnit(
                self.raw,
                language.to_raw(),
                file.raw,
                producer.as_ptr().cast(),
                producer.len(),
                i32::from(is_optimized),
                flags.as_ptr().cast(),
                flags.len(),
                runtime_version,
                split_name.as_ptr().cast(),
                split_name.len(),
                kind.to_raw(),
                dwarf_object_identifier,
                i32::from(split_debug_inlining),
                i32::from(debug_info_for_profiling),
                system_root.as_ptr().cast(),
                system_root.len(),
                software_development_kit.as_ptr().cast(),
                software_development_kit.len(),
            ))
        }
    }

    /// A file.
    #[must_use]
    pub fn create_file(&self, file_name: &str, directory: &str) -> Metadata<'context, 'module> {
        // SAFETY: LLVM reads the lengths given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateFile(
                self.raw,
                file_name.as_ptr().cast(),
                file_name.len(),
                directory.as_ptr().cast(),
                directory.len(),
            ))
        }
    }

    /// A file with a checksum, and its source text if given.
    #[must_use]
    pub fn create_file_with_checksum(
        &self,
        file_name: &str,
        directory: &str,
        checksum_kind: ChecksumKind,
        checksum: &str,
        source: &str,
    ) -> Metadata<'context, 'module> {
        // SAFETY: LLVM reads the lengths given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateFileWithChecksum(
                self.raw,
                file_name.as_ptr().cast(),
                file_name.len(),
                directory.as_ptr().cast(),
                directory.len(),
                checksum_kind.to_raw(),
                checksum.as_ptr().cast(),
                checksum.len(),
                source.as_ptr().cast(),
                source.len(),
            ))
        }
    }

    /// A module, as a language's module is.
    ///
    /// # Panics
    /// If the parent isn't a scope (D3).
    #[must_use]
    #[track_caller]
    pub fn create_module(
        &self,
        parent_scope: Metadata<'context, 'module>,
        name: &str,
        configuration_macros: &str,
        include_path: &str,
        api_notes_file: &str,
    ) -> Metadata<'context, 'module> {
        check("create_module", parent_scope, Class::Scope);
        // SAFETY: just checked; LLVM reads the lengths given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateModule(
                self.raw,
                parent_scope.raw,
                name.as_ptr().cast(),
                name.len(),
                configuration_macros.as_ptr().cast(),
                configuration_macros.len(),
                include_path.as_ptr().cast(),
                include_path.len(),
                api_notes_file.as_ptr().cast(),
                api_notes_file.len(),
            ))
        }
    }

    /// A namespace.
    ///
    /// # Panics
    /// If the parent isn't a scope (D3).
    #[must_use]
    #[track_caller]
    pub fn create_namespace(
        &self,
        parent_scope: Metadata<'context, 'module>,
        name: &str,
        export_symbols: bool,
    ) -> Metadata<'context, 'module> {
        check("create_namespace", parent_scope, Class::Scope);
        // SAFETY: just checked; LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateNameSpace(
                self.raw,
                parent_scope.raw,
                name.as_ptr().cast(),
                name.len(),
                i32::from(export_symbols),
            ))
        }
    }

    /// A subprogram: a function, or its declaration.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_function(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        linkage_name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        function_type: Metadata<'context, 'module>,
        is_local_to_unit: bool,
        is_definition: bool,
        scope_line: u32,
        flags: DebugInfoFlags,
        is_optimized: bool,
    ) -> Metadata<'context, 'module> {
        check("create_function", scope, Class::Scope);
        check("create_function", file, Class::File);
        check("create_function", function_type, Class::Type);
        // SAFETY: just checked; LLVM reads the lengths given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateFunction(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                linkage_name.as_ptr().cast(),
                linkage_name.len(),
                file.raw,
                line_number,
                function_type.raw,
                i32::from(is_local_to_unit),
                i32::from(is_definition),
                scope_line,
                flags.to_raw(),
                i32::from(is_optimized),
            ))
        }
    }

    /// A lexical block.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_lexical_block(
        &self,
        scope: Metadata<'context, 'module>,
        file: Metadata<'context, 'module>,
        line: u32,
        column: u32,
    ) -> Metadata<'context, 'module> {
        check("create_lexical_block", scope, Class::Scope);
        check("create_lexical_block", file, Class::File);
        // SAFETY: just checked.
        unsafe { Metadata::from_raw(raw::LLVMDIBuilderCreateLexicalBlock(self.raw, scope.raw, file.raw, line, column)) }
    }

    /// A lexical block file: part of a block in another file.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_lexical_block_file(
        &self,
        scope: Metadata<'context, 'module>,
        file: Metadata<'context, 'module>,
        discriminator: u32,
    ) -> Metadata<'context, 'module> {
        check("create_lexical_block_file", scope, Class::Scope);
        check("create_lexical_block_file", file, Class::File);
        // SAFETY: just checked.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateLexicalBlockFile(self.raw, scope.raw, file.raw, discriminator))
        }
    }

    /// An import of a namespace.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_imported_module_from_namespace(
        &self,
        scope: Metadata<'context, 'module>,
        namespace: Metadata<'context, 'module>,
        file: Metadata<'context, 'module>,
        line: u32,
    ) -> Metadata<'context, 'module> {
        check("create_imported_module_from_namespace", scope, Class::Scope);
        check("create_imported_module_from_namespace", namespace, Class::Scope);
        check("create_imported_module_from_namespace", file, Class::File);
        // SAFETY: just checked.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateImportedModuleFromNamespace(
                self.raw,
                scope.raw,
                namespace.raw,
                file.raw,
                line,
            ))
        }
    }

    /// An import of a module, through an alias.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_imported_module_from_alias(
        &self,
        scope: Metadata<'context, 'module>,
        imported_entity: Metadata<'context, 'module>,
        file: Metadata<'context, 'module>,
        line: u32,
        elements: &[Metadata<'context, 'module>],
    ) -> Metadata<'context, 'module> {
        check("create_imported_module_from_alias", scope, Class::Scope);
        check("create_imported_module_from_alias", imported_entity, Class::DebugInfoNode);
        check("create_imported_module_from_alias", file, Class::File);
        let count = length_as_u32(elements.len(), "create_imported_module_from_alias");
        // SAFETY: just checked; LLVM only reads the elements, and `Metadata` has the layout of `LLVMMetadataRef`.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateImportedModuleFromAlias(
                self.raw,
                scope.raw,
                imported_entity.raw,
                file.raw,
                line,
                elements.as_ptr().cast_mut().cast(),
                count,
            ))
        }
    }

    /// An import of a module.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_imported_module_from_module(
        &self,
        scope: Metadata<'context, 'module>,
        imported_module: Metadata<'context, 'module>,
        file: Metadata<'context, 'module>,
        line: u32,
        elements: &[Metadata<'context, 'module>],
    ) -> Metadata<'context, 'module> {
        check("create_imported_module_from_module", scope, Class::Scope);
        check("create_imported_module_from_module", imported_module, Class::Scope);
        check("create_imported_module_from_module", file, Class::File);
        let count = length_as_u32(elements.len(), "create_imported_module_from_module");
        // SAFETY: just checked; LLVM only reads the elements, and `Metadata` has the layout of `LLVMMetadataRef`.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateImportedModuleFromModule(
                self.raw,
                scope.raw,
                imported_module.raw,
                file.raw,
                line,
                elements.as_ptr().cast_mut().cast(),
                count,
            ))
        }
    }

    /// An import of one declaration, under a name if given.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_imported_declaration(
        &self,
        scope: Metadata<'context, 'module>,
        declaration: Metadata<'context, 'module>,
        file: Metadata<'context, 'module>,
        line: u32,
        name: &str,
        elements: &[Metadata<'context, 'module>],
    ) -> Metadata<'context, 'module> {
        check("create_imported_declaration", scope, Class::Scope);
        check("create_imported_declaration", declaration, Class::DebugInfoNode);
        check("create_imported_declaration", file, Class::File);
        let count = length_as_u32(elements.len(), "create_imported_declaration");
        // SAFETY: just checked; LLVM only reads the elements and the length of the name.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateImportedDeclaration(
                self.raw,
                scope.raw,
                declaration.raw,
                file.raw,
                line,
                name.as_ptr().cast(),
                name.len(),
                elements.as_ptr().cast_mut().cast(),
                count,
            ))
        }
    }

    /// A debug location.
    ///
    /// # Panics
    /// If the scope isn't a local scope, or `inlined_at` isn't a location (D3).
    #[must_use]
    #[track_caller]
    pub fn create_debug_location(
        &self,
        line: u32,
        column: u32,
        scope: Metadata<'context, 'module>,
        inlined_at: Option<Metadata<'context, 'module>>,
    ) -> Metadata<'context, 'module> {
        check("create_debug_location", scope, Class::LocalScope);
        let raw_inlined = check_optional("create_debug_location", inlined_at, Class::Location);
        // SAFETY: just checked; a null `inlined_at` means it wasn't inlined.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateDebugLocation(
                self.context,
                line,
                column,
                scope.raw,
                raw_inlined,
            ))
        }
    }

    /// A type array, as a function type wants its types.
    #[must_use]
    pub fn get_or_create_type_array(&self, types: &[Metadata<'context, 'module>]) -> Metadata<'context, 'module> {
        // SAFETY: LLVM only reads them, and `Metadata` has the layout of `LLVMMetadataRef`.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderGetOrCreateTypeArray(
                self.raw,
                types.as_ptr().cast_mut().cast(),
                types.len(),
            ))
        }
    }

    /// A list of pieces, as a composite type wants its elements.
    #[must_use]
    pub fn get_or_create_array(&self, elements: &[Metadata<'context, 'module>]) -> Metadata<'context, 'module> {
        // SAFETY: LLVM only reads them, and `Metadata` has the layout of `LLVMMetadataRef`.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderGetOrCreateArray(
                self.raw,
                elements.as_ptr().cast_mut().cast(),
                elements.len(),
            ))
        }
    }

    /// A function's type: the return type, then the parameter types. An empty slot is `void`.
    ///
    /// # Panics
    /// If `file` isn't a file, or a parameter isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn create_subroutine_type(
        &self,
        file: Metadata<'context, 'module>,
        parameter_types: &[Metadata<'context, 'module>],
        flags: DebugInfoFlags,
    ) -> Metadata<'context, 'module> {
        check("create_subroutine_type", file, Class::File);
        check_list("create_subroutine_type", parameter_types, Class::Type);
        let count = length_as_u32(parameter_types.len(), "create_subroutine_type");
        // SAFETY: just checked; LLVM only reads the types, and `Metadata` has the layout of `LLVMMetadataRef`.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateSubroutineType(
                self.raw,
                file.raw,
                parameter_types.as_ptr().cast_mut().cast(),
                count,
                flags.to_raw(),
            ))
        }
    }

    /// A macro.
    ///
    /// # Panics
    /// If the parent isn't a macro file (D3).
    #[must_use]
    #[track_caller]
    pub fn create_macro(
        &self,
        parent_macro_file: Option<Metadata<'context, 'module>>,
        line: u32,
        record_type: DwarfMacroRecordType,
        name: &str,
        value: &str,
    ) -> Metadata<'context, 'module> {
        let raw_parent = check_optional("create_macro", parent_macro_file, Class::MacroFile);
        // SAFETY: just checked; LLVM reads the lengths given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateMacro(
                self.raw,
                raw_parent,
                line,
                record_type.to_raw(),
                name.as_ptr().cast(),
                name.len(),
                value.as_ptr().cast(),
                value.len(),
            ))
        }
    }

    /// A macro file, for its macros to go in.
    ///
    /// # Panics
    /// If the parent isn't a macro file, or `file` isn't a file (D3).
    #[must_use]
    #[track_caller]
    pub fn create_temporary_macro_file(
        &self,
        parent_macro_file: Option<Metadata<'context, 'module>>,
        line: u32,
        file: Metadata<'context, 'module>,
    ) -> Metadata<'context, 'module> {
        let raw_parent = check_optional("create_temporary_macro_file", parent_macro_file, Class::MacroFile);
        check("create_temporary_macro_file", file, Class::File);
        // SAFETY: just checked.
        unsafe { Metadata::from_raw(raw::LLVMDIBuilderCreateTempMacroFile(self.raw, raw_parent, line, file.raw)) }
    }

    /// An enumerator.
    #[must_use]
    pub fn create_enumerator(&self, name: &str, value: i64, is_unsigned: bool) -> Metadata<'context, 'module> {
        // SAFETY: LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateEnumerator(
                self.raw,
                name.as_ptr().cast(),
                name.len(),
                value,
                i32::from(is_unsigned),
            ))
        }
    }

    /// An enumerator wider than 64 bits, from little-endian 64-bit words.
    ///
    /// # Panics
    /// If there aren't enough words for the size (D3).
    #[must_use]
    #[track_caller]
    pub fn create_enumerator_of_arbitrary_precision(
        &self,
        name: &str,
        size_in_bits: u64,
        words: &[u64],
        is_unsigned: bool,
    ) -> Metadata<'context, 'module> {
        let needed = size_in_bits.div_ceil(64);
        if !checks::enough_words(words.len(), usize::try_from(needed).unwrap_or(usize::MAX)) {
            bad_call(format_args!(
                "create_enumerator_of_arbitrary_precision: {size_in_bits} bits needs {needed} words, not {}",
                words.len()
            ));
        }
        // SAFETY: the words were just counted; LLVM reads the lengths given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateEnumeratorOfArbitraryPrecision(
                self.raw,
                name.as_ptr().cast(),
                name.len(),
                size_in_bits,
                words.as_ptr(),
                i32::from(is_unsigned),
            ))
        }
    }

    /// An enumeration type.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_enumeration_type(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        size_in_bits: u64,
        alignment_in_bits: u32,
        elements: &[Metadata<'context, 'module>],
        class_type: Option<Metadata<'context, 'module>>,
    ) -> Metadata<'context, 'module> {
        check("create_enumeration_type", scope, Class::Scope);
        check("create_enumeration_type", file, Class::File);
        let raw_class = check_optional("create_enumeration_type", class_type, Class::Type);
        let count = length_as_u32(elements.len(), "create_enumeration_type");
        // SAFETY: just checked; LLVM only reads the elements, and the length of the name.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateEnumerationType(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                size_in_bits,
                alignment_in_bits,
                elements.as_ptr().cast_mut().cast(),
                count,
                raw_class,
            ))
        }
    }

    /// A union type.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_union_type(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        size_in_bits: u64,
        alignment_in_bits: u32,
        flags: DebugInfoFlags,
        elements: &[Metadata<'context, 'module>],
        runtime_language: u32,
        unique_identifier: &str,
    ) -> Metadata<'context, 'module> {
        check("create_union_type", scope, Class::Scope);
        check("create_union_type", file, Class::File);
        let count = length_as_u32(elements.len(), "create_union_type");
        // SAFETY: just checked; LLVM only reads the elements, and the lengths of the text.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateUnionType(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                size_in_bits,
                alignment_in_bits,
                flags.to_raw(),
                elements.as_ptr().cast_mut().cast(),
                count,
                runtime_language,
                unique_identifier.as_ptr().cast(),
                unique_identifier.len(),
            ))
        }
    }

    /// An array type. `subscripts` holds subranges.
    ///
    /// # Panics
    /// If the element isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn create_array_type(
        &self,
        size_in_bits: u64,
        alignment_in_bits: u32,
        element_type: Metadata<'context, 'module>,
        subscripts: &[Metadata<'context, 'module>],
    ) -> Metadata<'context, 'module> {
        check("create_array_type", element_type, Class::Type);
        check_list("create_array_type", subscripts, Class::DebugInfoNode);
        let count = length_as_u32(subscripts.len(), "create_array_type");
        // SAFETY: just checked; LLVM only reads the subscripts.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateArrayType(
                self.raw,
                size_in_bits,
                alignment_in_bits,
                element_type.raw,
                subscripts.as_ptr().cast_mut().cast(),
                count,
            ))
        }
    }

    /// A set type.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_set_type(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        size_in_bits: u64,
        alignment_in_bits: u32,
        base_type: Metadata<'context, 'module>,
    ) -> Metadata<'context, 'module> {
        check("create_set_type", scope, Class::Scope);
        check("create_set_type", file, Class::File);
        check("create_set_type", base_type, Class::Type);
        // SAFETY: just checked; LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateSetType(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                size_in_bits,
                alignment_in_bits,
                base_type.raw,
            ))
        }
    }

    /// A subrange type, as a Pascal subrange is.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_subrange_type(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        line_number: u32,
        file: Metadata<'context, 'module>,
        size_in_bits: u64,
        alignment_in_bits: u32,
        flags: DebugInfoFlags,
        base_type: Metadata<'context, 'module>,
        lower_bound: Metadata<'context, 'module>,
        upper_bound: Metadata<'context, 'module>,
        stride: Metadata<'context, 'module>,
        bias: Metadata<'context, 'module>,
    ) -> Metadata<'context, 'module> {
        check("create_subrange_type", scope, Class::Scope);
        check("create_subrange_type", file, Class::File);
        check("create_subrange_type", base_type, Class::Type);
        // SAFETY: just checked; LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateSubrangeType(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                line_number,
                file.raw,
                size_in_bits,
                alignment_in_bits,
                flags.to_raw(),
                base_type.raw,
                lower_bound.raw,
                upper_bound.raw,
                stride.raw,
                bias.raw,
            ))
        }
    }

    /// An array type whose shape is only known when the program runs.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_dynamic_array_type(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        line_number: u32,
        file: Metadata<'context, 'module>,
        size_in_bits: u64,
        alignment_in_bits: u32,
        element_type: Metadata<'context, 'module>,
        subscripts: &[Metadata<'context, 'module>],
        data_location: Metadata<'context, 'module>,
        associated: Metadata<'context, 'module>,
        allocated: Metadata<'context, 'module>,
        rank: Metadata<'context, 'module>,
        bit_stride: Metadata<'context, 'module>,
    ) -> Metadata<'context, 'module> {
        check("create_dynamic_array_type", scope, Class::Scope);
        check("create_dynamic_array_type", file, Class::File);
        check("create_dynamic_array_type", element_type, Class::Type);
        let count = length_as_u32(subscripts.len(), "create_dynamic_array_type");
        // SAFETY: just checked; LLVM only reads the subscripts, and the length of the name.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateDynamicArrayType(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                line_number,
                file.raw,
                size_in_bits,
                alignment_in_bits,
                element_type.raw,
                subscripts.as_ptr().cast_mut().cast(),
                count,
                data_location.raw,
                associated.raw,
                allocated.raw,
                rank.raw,
                bit_stride.raw,
            ))
        }
    }

    /// A vector type.
    ///
    /// # Panics
    /// If the element isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn create_vector_type(
        &self,
        size_in_bits: u64,
        alignment_in_bits: u32,
        element_type: Metadata<'context, 'module>,
        subscripts: &[Metadata<'context, 'module>],
    ) -> Metadata<'context, 'module> {
        check("create_vector_type", element_type, Class::Type);
        let count = length_as_u32(subscripts.len(), "create_vector_type");
        // SAFETY: just checked; LLVM only reads the subscripts.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateVectorType(
                self.raw,
                size_in_bits,
                alignment_in_bits,
                element_type.raw,
                subscripts.as_ptr().cast_mut().cast(),
                count,
            ))
        }
    }

    /// A type that has no description, such as `decltype(nullptr)`.
    #[must_use]
    pub fn create_unspecified_type(&self, name: &str) -> Metadata<'context, 'module> {
        // SAFETY: LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateUnspecifiedType(self.raw, name.as_ptr().cast(), name.len()))
        }
    }

    /// A basic type, whose `encoding` is a DWARF `DW_ATE_` number.
    #[must_use]
    pub fn create_basic_type(
        &self,
        name: &str,
        size_in_bits: u64,
        encoding: u32,
        flags: DebugInfoFlags,
    ) -> Metadata<'context, 'module> {
        // SAFETY: LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateBasicType(
                self.raw,
                name.as_ptr().cast(),
                name.len(),
                size_in_bits,
                encoding,
                flags.to_raw(),
            ))
        }
    }

    /// A pointer type.
    ///
    /// # Panics
    /// If the pointee isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn create_pointer_type(
        &self,
        pointee_type: Metadata<'context, 'module>,
        size_in_bits: u64,
        alignment_in_bits: u32,
        address_space: u32,
        name: &str,
    ) -> Metadata<'context, 'module> {
        check("create_pointer_type", pointee_type, Class::Type);
        // SAFETY: just checked; LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreatePointerType(
                self.raw,
                pointee_type.raw,
                size_in_bits,
                alignment_in_bits,
                address_space,
                name.as_ptr().cast(),
                name.len(),
            ))
        }
    }

    /// A struct type.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_struct_type(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        size_in_bits: u64,
        alignment_in_bits: u32,
        flags: DebugInfoFlags,
        derived_from: Option<Metadata<'context, 'module>>,
        elements: &[Metadata<'context, 'module>],
        runtime_language: u32,
        virtual_table_holder: Option<Metadata<'context, 'module>>,
        unique_identifier: &str,
    ) -> Metadata<'context, 'module> {
        check("create_struct_type", scope, Class::Scope);
        check("create_struct_type", file, Class::File);
        let raw_derived = check_optional("create_struct_type", derived_from, Class::Type);
        let raw_holder = check_optional("create_struct_type", virtual_table_holder, Class::Type);
        let count = length_as_u32(elements.len(), "create_struct_type");
        // SAFETY: just checked; LLVM only reads the elements, and the lengths of the text.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateStructType(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                size_in_bits,
                alignment_in_bits,
                flags.to_raw(),
                raw_derived,
                elements.as_ptr().cast_mut().cast(),
                count,
                runtime_language,
                raw_holder,
                unique_identifier.as_ptr().cast(),
                unique_identifier.len(),
            ))
        }
    }

    /// A member of a composite type.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_member_type(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        size_in_bits: u64,
        alignment_in_bits: u32,
        offset_in_bits: u64,
        flags: DebugInfoFlags,
        member_type: Metadata<'context, 'module>,
    ) -> Metadata<'context, 'module> {
        check("create_member_type", scope, Class::Scope);
        check("create_member_type", file, Class::File);
        check("create_member_type", member_type, Class::Type);
        // SAFETY: just checked; LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateMemberType(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                size_in_bits,
                alignment_in_bits,
                offset_in_bits,
                flags.to_raw(),
                member_type.raw,
            ))
        }
    }

    /// A static member of a composite type.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants, or the value isn't a constant (D3).
    #[must_use]
    #[track_caller]
    pub fn create_static_member_type<T>(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        member_type: Metadata<'context, 'module>,
        flags: DebugInfoFlags,
        constant_value: Option<Value<'context, 'module, T>>,
        alignment_in_bits: u32,
    ) -> Metadata<'context, 'module> {
        check("create_static_member_type", scope, Class::Scope);
        check("create_static_member_type", file, Class::File);
        check("create_static_member_type", member_type, Class::Type);
        let raw_constant = match constant_value {
            Some(found) => {
                if !found.is_constant() {
                    bad_call(format_args!("create_static_member_type: {} isn't a constant", found.print_to_string()));
                }
                found.raw
            }
            None => std::ptr::null_mut(),
        };
        // SAFETY: just checked; LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateStaticMemberType(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                member_type.raw,
                flags.to_raw(),
                raw_constant,
                alignment_in_bits,
            ))
        }
    }

    /// A pointer to a member.
    ///
    /// # Panics
    /// If a piece isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn create_member_pointer_type(
        &self,
        pointee_type: Metadata<'context, 'module>,
        class_type: Metadata<'context, 'module>,
        size_in_bits: u64,
        alignment_in_bits: u32,
        flags: DebugInfoFlags,
    ) -> Metadata<'context, 'module> {
        check("create_member_pointer_type", pointee_type, Class::Type);
        check("create_member_pointer_type", class_type, Class::Type);
        // SAFETY: just checked.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateMemberPointerType(
                self.raw,
                pointee_type.raw,
                class_type.raw,
                size_in_bits,
                alignment_in_bits,
                flags.to_raw(),
            ))
        }
    }

    /// An Objective-C instance variable.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_objective_c_instance_variable(
        &self,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        size_in_bits: u64,
        alignment_in_bits: u32,
        offset_in_bits: u64,
        flags: DebugInfoFlags,
        variable_type: Metadata<'context, 'module>,
        property_node: Metadata<'context, 'module>,
    ) -> Metadata<'context, 'module> {
        check("create_objective_c_instance_variable", file, Class::File);
        check("create_objective_c_instance_variable", variable_type, Class::Type);
        check("create_objective_c_instance_variable", property_node, Class::Node);
        // SAFETY: just checked; LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateObjCIVar(
                self.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                size_in_bits,
                alignment_in_bits,
                offset_in_bits,
                flags.to_raw(),
                variable_type.raw,
                property_node.raw,
            ))
        }
    }

    /// An Objective-C property.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_objective_c_property(
        &self,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        getter_name: &str,
        setter_name: &str,
        property_attributes: u32,
        property_type: Metadata<'context, 'module>,
    ) -> Metadata<'context, 'module> {
        check("create_objective_c_property", file, Class::File);
        check("create_objective_c_property", property_type, Class::Type);
        // SAFETY: just checked; LLVM reads the lengths given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateObjCProperty(
                self.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                getter_name.as_ptr().cast(),
                getter_name.len(),
                setter_name.as_ptr().cast(),
                setter_name.len(),
                property_attributes,
                property_type.raw,
            ))
        }
    }

    /// A type marked as `this`, or the like.
    ///
    /// # Panics
    /// If it isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn create_object_pointer_type(
        &self,
        object_type: Metadata<'context, 'module>,
        implicit: bool,
    ) -> Metadata<'context, 'module> {
        check("create_object_pointer_type", object_type, Class::Type);
        // SAFETY: just checked.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateObjectPointerType(
                self.raw,
                object_type.raw,
                i32::from(implicit),
            ))
        }
    }

    /// A type with a qualifier, whose `tag` is a DWARF `DW_TAG_` number such as `const_type`.
    ///
    /// # Panics
    /// If it isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn create_qualified_type(&self, tag: u32, base: Metadata<'context, 'module>) -> Metadata<'context, 'module> {
        check("create_qualified_type", base, Class::Type);
        // SAFETY: just checked.
        unsafe { Metadata::from_raw(raw::LLVMDIBuilderCreateQualifiedType(self.raw, tag, base.raw)) }
    }

    /// A reference type, whose `tag` is a DWARF `DW_TAG_` number such as `reference_type`.
    ///
    /// # Panics
    /// If it isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn create_reference_type(&self, tag: u32, base: Metadata<'context, 'module>) -> Metadata<'context, 'module> {
        check("create_reference_type", base, Class::Type);
        // SAFETY: just checked.
        unsafe { Metadata::from_raw(raw::LLVMDIBuilderCreateReferenceType(self.raw, tag, base.raw)) }
    }

    /// The type of C++'s `nullptr`.
    #[must_use]
    pub fn create_null_pointer_type(&self) -> Metadata<'context, 'module> {
        // SAFETY: the builder is live.
        unsafe { Metadata::from_raw(raw::LLVMDIBuilderCreateNullPtrType(self.raw)) }
    }

    /// A type's other name.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_typedef(
        &self,
        base: Metadata<'context, 'module>,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        scope: Metadata<'context, 'module>,
        alignment_in_bits: u32,
    ) -> Metadata<'context, 'module> {
        check("create_typedef", base, Class::Type);
        check("create_typedef", file, Class::File);
        check("create_typedef", scope, Class::Scope);
        // SAFETY: just checked; LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateTypedef(
                self.raw,
                base.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                scope.raw,
                alignment_in_bits,
            ))
        }
    }

    /// A type's base class.
    ///
    /// # Panics
    /// If a piece isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn create_inheritance(
        &self,
        derived: Metadata<'context, 'module>,
        base: Metadata<'context, 'module>,
        base_offset_in_bits: u64,
        virtual_base_pointer_offset_in_bits: u32,
        flags: DebugInfoFlags,
    ) -> Metadata<'context, 'module> {
        check("create_inheritance", derived, Class::Type);
        check("create_inheritance", base, Class::Type);
        // SAFETY: just checked.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateInheritance(
                self.raw,
                derived.raw,
                base.raw,
                base_offset_in_bits,
                virtual_base_pointer_offset_in_bits,
                flags.to_raw(),
            ))
        }
    }

    /// A type declared but not defined, whose `tag` is a DWARF `DW_TAG_` number.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_forward_declaration(
        &self,
        tag: u32,
        name: &str,
        scope: Metadata<'context, 'module>,
        file: Metadata<'context, 'module>,
        line: u32,
        runtime_language: u32,
        size_in_bits: u64,
        alignment_in_bits: u32,
        unique_identifier: &str,
    ) -> Metadata<'context, 'module> {
        check("create_forward_declaration", scope, Class::Scope);
        check("create_forward_declaration", file, Class::File);
        // SAFETY: just checked; LLVM reads the lengths given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateForwardDecl(
                self.raw,
                tag,
                name.as_ptr().cast(),
                name.len(),
                scope.raw,
                file.raw,
                line,
                runtime_language,
                size_in_bits,
                alignment_in_bits,
                unique_identifier.as_ptr().cast(),
                unique_identifier.len(),
            ))
        }
    }

    /// A temporary composite type, to replace with the real one when it is known.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_replaceable_composite_type(
        &self,
        tag: u32,
        name: &str,
        scope: Metadata<'context, 'module>,
        file: Metadata<'context, 'module>,
        line: u32,
        runtime_language: u32,
        size_in_bits: u64,
        alignment_in_bits: u32,
        flags: DebugInfoFlags,
        unique_identifier: &str,
    ) -> TemporaryMetadata<'context, 'module> {
        check("create_replaceable_composite_type", scope, Class::Scope);
        check("create_replaceable_composite_type", file, Class::File);
        // SAFETY: just checked; LLVM reads the lengths given, and makes a temporary node the wrapper owns.
        let raw_node = unsafe {
            raw::LLVMDIBuilderCreateReplaceableCompositeType(
                self.raw,
                tag,
                name.as_ptr().cast(),
                name.len(),
                scope.raw,
                file.raw,
                line,
                runtime_language,
                size_in_bits,
                alignment_in_bits,
                flags.to_raw(),
                unique_identifier.as_ptr().cast(),
                unique_identifier.len(),
            )
        };
        TemporaryMetadata { raw: raw_node, marker: PhantomData }
    }

    /// A member that takes only some of the bits of its storage.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_bit_field_member_type(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        size_in_bits: u64,
        offset_in_bits: u64,
        storage_offset_in_bits: u64,
        flags: DebugInfoFlags,
        member_type: Metadata<'context, 'module>,
    ) -> Metadata<'context, 'module> {
        check("create_bit_field_member_type", scope, Class::Scope);
        check("create_bit_field_member_type", file, Class::File);
        check("create_bit_field_member_type", member_type, Class::Type);
        // SAFETY: just checked; LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateBitFieldMemberType(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                size_in_bits,
                offset_in_bits,
                storage_offset_in_bits,
                flags.to_raw(),
                member_type.raw,
            ))
        }
    }

    /// A class type.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_class_type(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        size_in_bits: u64,
        alignment_in_bits: u32,
        offset_in_bits: u64,
        flags: DebugInfoFlags,
        derived_from: Option<Metadata<'context, 'module>>,
        elements: &[Metadata<'context, 'module>],
        virtual_table_holder: Option<Metadata<'context, 'module>>,
        template_parameters: Option<Metadata<'context, 'module>>,
        unique_identifier: &str,
    ) -> Metadata<'context, 'module> {
        check("create_class_type", scope, Class::Scope);
        check("create_class_type", file, Class::File);
        let raw_derived = check_optional("create_class_type", derived_from, Class::Type);
        let raw_holder = check_optional("create_class_type", virtual_table_holder, Class::Type);
        let raw_template = check_optional("create_class_type", template_parameters, Class::Node);
        let count = length_as_u32(elements.len(), "create_class_type");
        // SAFETY: just checked; LLVM only reads the elements, and the lengths of the text.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateClassType(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                size_in_bits,
                alignment_in_bits,
                offset_in_bits,
                flags.to_raw(),
                raw_derived,
                elements.as_ptr().cast_mut().cast(),
                count,
                raw_holder,
                raw_template,
                unique_identifier.as_ptr().cast(),
                unique_identifier.len(),
            ))
        }
    }

    /// A type marked as made by the compiler.
    ///
    /// # Panics
    /// If it isn't a type (D3).
    #[must_use]
    #[track_caller]
    pub fn create_artificial_type(&self, base: Metadata<'context, 'module>) -> Metadata<'context, 'module> {
        check("create_artificial_type", base, Class::Type);
        // SAFETY: just checked.
        unsafe { Metadata::from_raw(raw::LLVMDIBuilderCreateArtificialType(self.raw, base.raw)) }
    }

    /// A subrange, as an array type wants its subscripts.
    #[must_use]
    pub fn get_or_create_subrange(&self, lower_bound: i64, count: i64) -> Metadata<'context, 'module> {
        // SAFETY: the builder is live.
        unsafe { Metadata::from_raw(raw::LLVMDIBuilderGetOrCreateSubrange(self.raw, lower_bound, count)) }
    }

    /// A location expression from DWARF operations.
    #[must_use]
    pub fn create_expression(&self, operations: &[u64]) -> Metadata<'context, 'module> {
        // SAFETY: LLVM only reads them. Its signature wants a mutable pointer by old habit.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateExpression(
                self.raw,
                operations.as_ptr().cast_mut(),
                operations.len(),
            ))
        }
    }

    /// A location expression that is a constant.
    #[must_use]
    pub fn create_constant_value_expression(&self, value: u64) -> Metadata<'context, 'module> {
        // SAFETY: the builder is live.
        unsafe { Metadata::from_raw(raw::LLVMDIBuilderCreateConstantValueExpression(self.raw, value)) }
    }

    /// A global variable, with the expression for where it is.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_global_variable_expression(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        linkage_name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        variable_type: Metadata<'context, 'module>,
        is_local_to_unit: bool,
        expression: Metadata<'context, 'module>,
        declaration: Option<Metadata<'context, 'module>>,
        alignment_in_bits: u32,
    ) -> Metadata<'context, 'module> {
        check("create_global_variable_expression", scope, Class::Scope);
        check("create_global_variable_expression", file, Class::File);
        check("create_global_variable_expression", variable_type, Class::Type);
        check("create_global_variable_expression", expression, Class::Expression);
        let raw_declaration = check_optional("create_global_variable_expression", declaration, Class::Node);
        // SAFETY: just checked; LLVM reads the lengths given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateGlobalVariableExpression(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                linkage_name.as_ptr().cast(),
                linkage_name.len(),
                file.raw,
                line_number,
                variable_type.raw,
                i32::from(is_local_to_unit),
                expression.raw,
                raw_declaration,
                alignment_in_bits,
            ))
        }
    }

    /// A temporary global variable declaration.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_temporary_global_variable_forward_declaration(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        linkage_name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        variable_type: Metadata<'context, 'module>,
        is_local_to_unit: bool,
        declaration: Option<Metadata<'context, 'module>>,
        alignment_in_bits: u32,
    ) -> TemporaryMetadata<'context, 'module> {
        check("create_temporary_global_variable_forward_declaration", scope, Class::Scope);
        check("create_temporary_global_variable_forward_declaration", file, Class::File);
        check("create_temporary_global_variable_forward_declaration", variable_type, Class::Type);
        let raw_declaration =
            check_optional("create_temporary_global_variable_forward_declaration", declaration, Class::Node);
        // SAFETY: just checked; LLVM reads the lengths given, and makes a temporary node the wrapper owns.
        let raw_node = unsafe {
            raw::LLVMDIBuilderCreateTempGlobalVariableFwdDecl(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                linkage_name.as_ptr().cast(),
                linkage_name.len(),
                file.raw,
                line_number,
                variable_type.raw,
                i32::from(is_local_to_unit),
                raw_declaration,
                alignment_in_bits,
            )
        };
        TemporaryMetadata { raw: raw_node, marker: PhantomData }
    }

    /// A local variable.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_auto_variable(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        variable_type: Metadata<'context, 'module>,
        always_preserve: bool,
        flags: DebugInfoFlags,
        alignment_in_bits: u32,
    ) -> Metadata<'context, 'module> {
        check("create_auto_variable", scope, Class::LocalScope);
        check("create_auto_variable", file, Class::File);
        check("create_auto_variable", variable_type, Class::Type);
        // SAFETY: just checked; LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateAutoVariable(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                variable_type.raw,
                i32::from(always_preserve),
                flags.to_raw(),
                alignment_in_bits,
            ))
        }
    }

    /// A function's parameter, whose `argument_number` starts at 1.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants, or the number is zero (D3).
    #[must_use]
    #[track_caller]
    pub fn create_parameter_variable(
        &self,
        scope: Metadata<'context, 'module>,
        name: &str,
        argument_number: u32,
        file: Metadata<'context, 'module>,
        line_number: u32,
        variable_type: Metadata<'context, 'module>,
        always_preserve: bool,
        flags: DebugInfoFlags,
    ) -> Metadata<'context, 'module> {
        check("create_parameter_variable", scope, Class::LocalScope);
        check("create_parameter_variable", file, Class::File);
        check("create_parameter_variable", variable_type, Class::Type);
        if argument_number == 0 {
            bad_call(format_args!("create_parameter_variable: argument numbers start at 1"));
        }
        // SAFETY: just checked; LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateParameterVariable(
                self.raw,
                scope.raw,
                name.as_ptr().cast(),
                name.len(),
                argument_number,
                file.raw,
                line_number,
                variable_type.raw,
                i32::from(always_preserve),
                flags.to_raw(),
            ))
        }
    }

    /// A label.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[must_use]
    #[track_caller]
    pub fn create_label(
        &self,
        context: Metadata<'context, 'module>,
        name: &str,
        file: Metadata<'context, 'module>,
        line_number: u32,
        always_preserve: bool,
    ) -> Metadata<'context, 'module> {
        check("create_label", context, Class::LocalScope);
        check("create_label", file, Class::File);
        // SAFETY: just checked; LLVM reads the length given.
        unsafe {
            Metadata::from_raw(raw::LLVMDIBuilderCreateLabel(
                self.raw,
                context.raw,
                name.as_ptr().cast(),
                name.len(),
                file.raw,
                line_number,
                i32::from(always_preserve),
            ))
        }
    }

    #[track_caller]
    fn check_record_pieces(
        &self,
        operation: &str,
        variable: Metadata<'context, 'module>,
        expression: Metadata<'context, 'module>,
        location: Metadata<'context, 'module>,
    ) {
        check(operation, variable, Class::LocalVariable);
        check(operation, expression, Class::Expression);
        check(operation, location, Class::Location);
    }

    /// Says where a variable lives, before an instruction.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[track_caller]
    pub fn insert_declare_before<S, I: InstructionMarker>(
        &self,
        storage: Value<'context, 'module, S>,
        variable: Metadata<'context, 'module>,
        expression: Metadata<'context, 'module>,
        location: Metadata<'context, 'module>,
        instruction: Value<'context, 'module, I>,
    ) -> DebugRecord<'context, 'module> {
        self.check_record_pieces("insert_declare_before", variable, expression, location);
        // SAFETY: just checked, and both values are live for `'module`.
        DebugRecord {
            raw: unsafe {
                raw::LLVMDIBuilderInsertDeclareRecordBefore(
                    self.raw,
                    storage.raw,
                    variable.raw,
                    expression.raw,
                    location.raw,
                    instruction.raw,
                )
            },
            marker: PhantomData,
        }
    }

    /// Says where a variable lives, at the end of a block.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[track_caller]
    pub fn insert_declare_at_end<S>(
        &self,
        storage: Value<'context, 'module, S>,
        variable: Metadata<'context, 'module>,
        expression: Metadata<'context, 'module>,
        location: Metadata<'context, 'module>,
        block: Block<'context, 'module>,
    ) -> DebugRecord<'context, 'module> {
        self.check_record_pieces("insert_declare_at_end", variable, expression, location);
        // SAFETY: just checked, and both are live for `'module`.
        DebugRecord {
            raw: unsafe {
                raw::LLVMDIBuilderInsertDeclareRecordAtEnd(
                    self.raw,
                    storage.raw,
                    variable.raw,
                    expression.raw,
                    location.raw,
                    block.raw,
                )
            },
            marker: PhantomData,
        }
    }

    /// Says what a variable's value is, before an instruction.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[track_caller]
    pub fn insert_value_before<S, I: InstructionMarker>(
        &self,
        value: Value<'context, 'module, S>,
        variable: Metadata<'context, 'module>,
        expression: Metadata<'context, 'module>,
        location: Metadata<'context, 'module>,
        instruction: Value<'context, 'module, I>,
    ) -> DebugRecord<'context, 'module> {
        self.check_record_pieces("insert_value_before", variable, expression, location);
        // SAFETY: just checked, and both values are live for `'module`.
        DebugRecord {
            raw: unsafe {
                raw::LLVMDIBuilderInsertDbgValueRecordBefore(
                    self.raw,
                    value.raw,
                    variable.raw,
                    expression.raw,
                    location.raw,
                    instruction.raw,
                )
            },
            marker: PhantomData,
        }
    }

    /// Says what a variable's value is, at the end of a block.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[track_caller]
    pub fn insert_value_at_end<S>(
        &self,
        value: Value<'context, 'module, S>,
        variable: Metadata<'context, 'module>,
        expression: Metadata<'context, 'module>,
        location: Metadata<'context, 'module>,
        block: Block<'context, 'module>,
    ) -> DebugRecord<'context, 'module> {
        self.check_record_pieces("insert_value_at_end", variable, expression, location);
        // SAFETY: just checked, and both are live for `'module`.
        DebugRecord {
            raw: unsafe {
                raw::LLVMDIBuilderInsertDbgValueRecordAtEnd(
                    self.raw,
                    value.raw,
                    variable.raw,
                    expression.raw,
                    location.raw,
                    block.raw,
                )
            },
            marker: PhantomData,
        }
    }

    /// Says where a label is, before an instruction.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[track_caller]
    pub fn insert_label_before<I: InstructionMarker>(
        &self,
        label: Metadata<'context, 'module>,
        location: Metadata<'context, 'module>,
        instruction: Value<'context, 'module, I>,
    ) -> DebugRecord<'context, 'module> {
        check("insert_label_before", label, Class::Label);
        check("insert_label_before", location, Class::Location);
        // SAFETY: just checked, and the instruction is live for `'module`.
        DebugRecord {
            raw: unsafe { raw::LLVMDIBuilderInsertLabelBefore(self.raw, label.raw, location.raw, instruction.raw) },
            marker: PhantomData,
        }
    }

    /// Says where a label is, at the end of a block.
    ///
    /// # Panics
    /// If a piece isn't the class LLVM wants (D3).
    #[track_caller]
    pub fn insert_label_at_end(
        &self,
        label: Metadata<'context, 'module>,
        location: Metadata<'context, 'module>,
        block: Block<'context, 'module>,
    ) -> DebugRecord<'context, 'module> {
        check("insert_label_at_end", label, Class::Label);
        check("insert_label_at_end", location, Class::Location);
        // SAFETY: just checked, and the block is live for `'module`.
        DebugRecord {
            raw: unsafe { raw::LLVMDIBuilderInsertLabelAtEnd(self.raw, label.raw, location.raw, block.raw) },
            marker: PhantomData,
        }
    }

    /// Replaces a composite type's elements.
    ///
    /// # Panics
    /// If it isn't a type (D3).
    #[track_caller]
    pub fn replace_arrays(
        &self,
        composite: Metadata<'context, 'module>,
        elements: &[Metadata<'context, 'module>],
    ) -> Metadata<'context, 'module> {
        check("replace_arrays", composite, Class::Type);
        let count = length_as_u32(elements.len(), "replace_arrays");
        let mut raw_composite = composite.raw;
        // SAFETY: just checked; LLVM may give back a new node in the variable, and only reads the elements.
        unsafe {
            raw::LLVMReplaceArrays(self.raw, &raw mut raw_composite, elements.as_ptr().cast_mut().cast(), count);
            Metadata::from_raw(raw_composite)
        }
    }

    /// Ties a function to its subprogram.
    pub fn set_subprogram(
        &self,
        function: Value<'context, 'module, Function>,
        subprogram: Metadata<'context, 'module>,
    ) {
        check("set_subprogram", subprogram, Class::Subprogram);
        function.set_subprogram(subprogram);
    }

    /// Ties a global variable to its debug info expression.
    pub fn add_global_debug_info(
        &self,
        global: Value<'context, 'module, GlobalVariable>,
        expression: Metadata<'context, 'module>,
    ) {
        check("add_global_debug_info", expression, Class::GlobalVariableExpression);
        global.add_debug_info(expression);
    }
}
