//! Reading object files and other binaries.

use std::marker::PhantomData;

use crate::bad_call::bad_call;
use crate::context::Context;
use crate::error::Error;
use crate::memory_buffer::MemoryBuffer;
use crate::raw;
use crate::text::{from_borrowed, from_owned_message};

/// What kind of file a binary is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryType {
    /// A static library.
    Archive,
    /// A Mach-O file holding several architectures.
    MachOUniversal,
    /// A COFF import file.
    CoffImportFile,
    /// LLVM IR.
    Ir,
    /// A Windows resource file.
    WindowsResource,
    /// A COFF object.
    Coff,
    /// A 32-bit little-endian ELF object.
    Elf32LittleEndian,
    /// A 32-bit big-endian ELF object.
    Elf32BigEndian,
    /// A 64-bit little-endian ELF object.
    Elf64LittleEndian,
    /// A 64-bit big-endian ELF object.
    Elf64BigEndian,
    /// A 32-bit little-endian Mach-O object.
    MachO32LittleEndian,
    /// A 32-bit big-endian Mach-O object.
    MachO32BigEndian,
    /// A 64-bit little-endian Mach-O object.
    MachO64LittleEndian,
    /// A 64-bit big-endian Mach-O object.
    MachO64BigEndian,
    /// A WebAssembly object.
    WebAssembly,
    /// An offload bundle.
    Offload,
    /// A DirectX container.
    DirectXContainer,
    /// A kind this Lyftar doesn't name.
    Unknown(u32),
}

/// A binary read from a buffer, which it borrows.
pub struct Binary<'data> {
    raw: raw::LLVMBinaryRef,
    marker: PhantomData<&'data [u8]>,
}

impl Drop for Binary<'_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the binary.
        unsafe { raw::LLVMDisposeBinary(self.raw) };
    }
}

impl<'data> Binary<'data> {
    /// Reads a binary. A file holding LLVM IR needs a context, which goes in `context`.
    ///
    /// # Errors
    /// What is wrong with the file.
    pub fn new(buffer: &'data MemoryBuffer<'data>, context: Option<Context<'data>>) -> Result<Self, Error> {
        let raw_context = context.map_or(std::ptr::null_mut(), |found| found.raw);
        let mut message = std::ptr::null_mut();
        // SAFETY: the buffer lasts for `'data`, a null context is allowed, and LLVM writes a message for the caller to free
        // on failure.
        let binary = unsafe { raw::LLVMCreateBinary(buffer.raw, raw_context, &raw mut message) };
        if binary.is_null() {
            return Err(Error::from_owned_message(message));
        }
        Ok(Binary { raw: binary, marker: PhantomData })
    }

    /// A copy of the bytes it was read from.
    #[must_use]
    pub fn copy_memory_buffer(&self) -> MemoryBuffer<'static> {
        // SAFETY: the binary is live, and LLVM makes a new buffer for the caller to own.
        unsafe { MemoryBuffer::from_raw(raw::LLVMBinaryCopyMemoryBuffer(self.raw)) }
    }

    /// What kind of file this is.
    #[must_use]
    pub fn binary_type(&self) -> BinaryType {
        use raw::LLVMBinaryType as T;
        // SAFETY: the binary is live.
        match unsafe { raw::LLVMBinaryGetType(self.raw) } {
            T::LLVMBinaryTypeArchive => BinaryType::Archive,
            T::LLVMBinaryTypeMachOUniversalBinary => BinaryType::MachOUniversal,
            T::LLVMBinaryTypeCOFFImportFile => BinaryType::CoffImportFile,
            T::LLVMBinaryTypeIR => BinaryType::Ir,
            T::LLVMBinaryTypeWinRes => BinaryType::WindowsResource,
            T::LLVMBinaryTypeCOFF => BinaryType::Coff,
            T::LLVMBinaryTypeELF32L => BinaryType::Elf32LittleEndian,
            T::LLVMBinaryTypeELF32B => BinaryType::Elf32BigEndian,
            T::LLVMBinaryTypeELF64L => BinaryType::Elf64LittleEndian,
            T::LLVMBinaryTypeELF64B => BinaryType::Elf64BigEndian,
            T::LLVMBinaryTypeMachO32L => BinaryType::MachO32LittleEndian,
            T::LLVMBinaryTypeMachO32B => BinaryType::MachO32BigEndian,
            T::LLVMBinaryTypeMachO64L => BinaryType::MachO64LittleEndian,
            T::LLVMBinaryTypeMachO64B => BinaryType::MachO64BigEndian,
            T::LLVMBinaryTypeWasm => BinaryType::WebAssembly,
            T::LLVMBinaryTypeOffload => BinaryType::Offload,
            T::LLVMBinaryTypeDXcontainer => BinaryType::DirectXContainer,
            T(other) => BinaryType::Unknown(other),
        }
    }

    /// From a Mach-O file holding several architectures, the one for `architecture`.
    ///
    /// # Errors
    /// If it has no such architecture.
    ///
    /// # Panics
    /// If this isn't such a file (D3).
    #[track_caller]
    pub fn macho_universal_object_for_architecture(&self, architecture: &str) -> Result<Binary<'data>, Error> {
        if self.binary_type() != BinaryType::MachOUniversal {
            bad_call(format_args!("macho_universal_object_for_architecture: this is a {:?}", self.binary_type()));
        }
        let mut message = std::ptr::null_mut();
        // SAFETY: just checked; LLVM reads the length given, and writes a message for the caller to free on failure.
        let binary = unsafe {
            raw::LLVMMachOUniversalBinaryCopyObjectForArch(
                self.raw,
                architecture.as_ptr().cast(),
                architecture.len(),
                &raw mut message,
            )
        };
        if binary.is_null() {
            return Err(Error::new(from_owned_message(message)));
        }
        Ok(Binary { raw: binary, marker: PhantomData })
    }

    #[track_caller]
    fn check_object_file(&self, operation: &str) {
        use BinaryType as T;
        let kind = self.binary_type();
        if !matches!(
            kind,
            T::Coff
                | T::Elf32LittleEndian
                | T::Elf32BigEndian
                | T::Elf64LittleEndian
                | T::Elf64BigEndian
                | T::MachO32LittleEndian
                | T::MachO32BigEndian
                | T::MachO64LittleEndian
                | T::MachO64BigEndian
                | T::WebAssembly
        ) {
            bad_call(format_args!("{operation}: a {kind:?} isn't an object file"));
        }
    }

    /// The sections of this object file.
    ///
    /// # Panics
    /// If this isn't an object file (D3).
    #[must_use]
    #[track_caller]
    pub fn sections(&self) -> SectionCursor<'_, 'data> {
        self.check_object_file("sections");
        // SAFETY: just checked; LLVM makes a cursor for the caller to own.
        SectionCursor { raw: unsafe { raw::LLVMObjectFileCopySectionIterator(self.raw) }, binary: self }
    }

    /// The symbols of this object file.
    ///
    /// # Panics
    /// If this isn't an object file (D3).
    #[must_use]
    #[track_caller]
    pub fn symbols(&self) -> SymbolCursor<'_, 'data> {
        self.check_object_file("symbols");
        // SAFETY: just checked; LLVM makes a cursor for the caller to own.
        SymbolCursor { raw: unsafe { raw::LLVMObjectFileCopySymbolIterator(self.raw) }, binary: self }
    }
}

/// A place among a binary's sections.
pub struct SectionCursor<'binary, 'data> {
    raw: raw::LLVMSectionIteratorRef,
    binary: &'binary Binary<'data>,
}

impl Drop for SectionCursor<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the cursor.
        unsafe { raw::LLVMDisposeSectionIterator(self.raw) };
    }
}

impl<'binary> SectionCursor<'binary, '_> {
    /// Whether this cursor is past the last section.
    #[must_use]
    pub fn is_at_end(&self) -> bool {
        // SAFETY: the cursor came from this binary, which is live.
        unsafe { raw::LLVMObjectFileIsSectionIteratorAtEnd(self.binary.raw, self.raw) != 0 }
    }

    #[track_caller]
    fn check_not_at_end(&self, operation: &str) {
        if self.is_at_end() {
            bad_call(format_args!("{operation}: this cursor is past the last section"));
        }
    }

    /// Moves to the next section.
    ///
    /// # Panics
    /// If this is already past the last one (D3).
    #[track_caller]
    pub fn move_to_next(&mut self) {
        self.check_not_at_end("move_to_next");
        // SAFETY: just checked.
        unsafe { raw::LLVMMoveToNextSection(self.raw) };
    }

    /// Moves to the section holding this symbol.
    pub fn move_to_containing_section(&mut self, symbol: &SymbolCursor<'_, '_>) {
        symbol.check_not_at_end("move_to_containing_section");
        // SAFETY: both are live, and the symbol was just checked.
        unsafe { raw::LLVMMoveToContainingSection(self.raw, symbol.raw) };
    }

    /// This section's name.
    ///
    /// # Panics
    /// If this is past the last one (D3).
    #[must_use]
    #[track_caller]
    pub fn name(&self) -> String {
        self.check_not_at_end("name");
        // SAFETY: just checked; the text is copied right away.
        from_borrowed(unsafe { raw::LLVMGetSectionName(self.raw) })
    }

    /// This section's size.
    ///
    /// # Panics
    /// If this is past the last one (D3).
    #[must_use]
    #[track_caller]
    pub fn size_in_bytes(&self) -> u64 {
        self.check_not_at_end("size_in_bytes");
        // SAFETY: just checked.
        unsafe { raw::LLVMGetSectionSize(self.raw) }
    }

    /// This section's address.
    ///
    /// # Panics
    /// If this is past the last one (D3).
    #[must_use]
    #[track_caller]
    pub fn address(&self) -> u64 {
        self.check_not_at_end("address");
        // SAFETY: just checked.
        unsafe { raw::LLVMGetSectionAddress(self.raw) }
    }

    /// This section's bytes.
    ///
    /// # Panics
    /// If this is past the last one (D3).
    #[must_use]
    #[track_caller]
    pub fn contents(&self) -> &'binary [u8] {
        let size = self.size_in_bytes();
        // SAFETY: just checked; a section's contents last as long as its binary, and its size says how many bytes they are.
        // A section with no bytes in the file gives a null pointer, which a slice can't be built from.
        unsafe {
            let contents = raw::LLVMGetSectionContents(self.raw);
            if contents.is_null() {
                return &[];
            }
            let Ok(length) = usize::try_from(size) else {
                bad_call(format_args!("contents: this section doesn't fit in memory"))
            };
            std::slice::from_raw_parts(contents.cast::<u8>(), length)
        }
    }

    /// Whether a symbol is in this section.
    ///
    /// # Panics
    /// If either cursor is past its end (D3).
    #[must_use]
    #[track_caller]
    pub fn contains_symbol(&self, symbol: &SymbolCursor<'_, '_>) -> bool {
        self.check_not_at_end("contains_symbol");
        symbol.check_not_at_end("contains_symbol");
        // SAFETY: just checked.
        unsafe { raw::LLVMGetSectionContainsSymbol(self.raw, symbol.raw) != 0 }
    }

    /// This section's relocations.
    ///
    /// # Panics
    /// If this is past the last one (D3).
    #[must_use]
    #[track_caller]
    pub fn relocations(&self) -> Vec<Relocation> {
        self.check_not_at_end("relocations");
        let mut relocations = Vec::new();
        // SAFETY: just checked; LLVM makes a cursor for the caller to free, walked only up to its end, and each relocation
        // is copied right away. The strings LLVM makes for a relocation are the caller's to free.
        unsafe {
            let cursor = raw::LLVMGetRelocations(self.raw);
            while raw::LLVMIsRelocationIteratorAtEnd(self.raw, cursor) == 0 {
                let symbol = raw::LLVMGetRelocationSymbol(cursor);
                let symbol_name =
                    if symbol.is_null() { String::new() } else { from_borrowed(raw::LLVMGetSymbolName(symbol)) };
                if !symbol.is_null() {
                    raw::LLVMDisposeSymbolIterator(symbol);
                }
                relocations.push(Relocation {
                    offset_in_bytes: raw::LLVMGetRelocationOffset(cursor),
                    symbol_name,
                    relocation_type: raw::LLVMGetRelocationType(cursor),
                    type_name: from_owned_message(raw::LLVMGetRelocationTypeName(cursor).cast_mut()),
                    value: from_owned_message(raw::LLVMGetRelocationValueString(cursor).cast_mut()),
                });
                raw::LLVMMoveToNextRelocation(cursor);
            }
            raw::LLVMDisposeRelocationIterator(cursor);
        }
        relocations
    }
}

/// One relocation in a section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relocation {
    /// Where it applies, from the section's start.
    pub offset_in_bytes: u64,
    /// The name of the symbol it refers to.
    pub symbol_name: String,
    /// The target's number for its type.
    pub relocation_type: u64,
    /// LLVM's name for its type.
    pub type_name: String,
    /// LLVM's text for its value.
    pub value: String,
}

/// A place among a binary's symbols.
pub struct SymbolCursor<'binary, 'data> {
    raw: raw::LLVMSymbolIteratorRef,
    binary: &'binary Binary<'data>,
}

impl Drop for SymbolCursor<'_, '_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the cursor.
        unsafe { raw::LLVMDisposeSymbolIterator(self.raw) };
    }
}

impl SymbolCursor<'_, '_> {
    /// Whether this cursor is past the last symbol.
    #[must_use]
    pub fn is_at_end(&self) -> bool {
        // SAFETY: the cursor came from this binary, which is live.
        unsafe { raw::LLVMObjectFileIsSymbolIteratorAtEnd(self.binary.raw, self.raw) != 0 }
    }

    #[track_caller]
    fn check_not_at_end(&self, operation: &str) {
        if self.is_at_end() {
            bad_call(format_args!("{operation}: this cursor is past the last symbol"));
        }
    }

    /// Moves to the next symbol.
    ///
    /// # Panics
    /// If this is already past the last one (D3).
    #[track_caller]
    pub fn move_to_next(&mut self) {
        self.check_not_at_end("move_to_next");
        // SAFETY: just checked.
        unsafe { raw::LLVMMoveToNextSymbol(self.raw) };
    }

    /// This symbol's name.
    ///
    /// # Panics
    /// If this is past the last one (D3).
    #[must_use]
    #[track_caller]
    pub fn name(&self) -> String {
        self.check_not_at_end("name");
        // SAFETY: just checked; the text is copied right away.
        from_borrowed(unsafe { raw::LLVMGetSymbolName(self.raw) })
    }

    /// This symbol's address.
    ///
    /// # Panics
    /// If this is past the last one (D3).
    #[must_use]
    #[track_caller]
    pub fn address(&self) -> u64 {
        self.check_not_at_end("address");
        // SAFETY: just checked.
        unsafe { raw::LLVMGetSymbolAddress(self.raw) }
    }

    /// This symbol's size.
    ///
    /// # Panics
    /// If this is past the last one (D3).
    #[must_use]
    #[track_caller]
    pub fn size_in_bytes(&self) -> u64 {
        self.check_not_at_end("size_in_bytes");
        // SAFETY: just checked.
        unsafe { raw::LLVMGetSymbolSize(self.raw) }
    }
}
