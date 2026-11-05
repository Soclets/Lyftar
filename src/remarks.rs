//! Reading optimization remarks.

use std::marker::PhantomData;

use crate::error::Error;
use crate::raw;
use crate::text::{from_borrowed, from_pointer_and_length};

/// What kind of remark a remark is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RemarkType {
    /// Unknown.
    Unknown,
    /// An optimization that happened.
    Passed,
    /// An optimization that didn't.
    Missed,
    /// Analysis.
    Analysis,
    /// Analysis about floating-point commutativity.
    AnalysisFloatCommute,
    /// Analysis about aliasing.
    AnalysisAliasing,
    /// A failure.
    Failure,
}

/// The LLVM remark format version this reads.
#[must_use]
pub fn version() -> u32 {
    // SAFETY: takes nothing.
    unsafe { raw::LLVMRemarkVersion() }
}

fn remark_string(string: raw::LLVMRemarkStringRef) -> String {
    if string.is_null() {
        return String::new();
    }
    // SAFETY: LLVM's remark string is live while its entry is, and this copies it right away.
    unsafe {
        from_pointer_and_length(raw::LLVMRemarkStringGetData(string), raw::LLVMRemarkStringGetLen(string) as usize)
    }
}

/// Where a remark is about, in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemarkDebugLocation {
    /// The source file's path.
    pub source_file_path: String,
    /// The line.
    pub line: u32,
    /// The column.
    pub column: u32,
}

fn debug_location(location: raw::LLVMRemarkDebugLocRef) -> Option<RemarkDebugLocation> {
    if location.is_null() {
        return None;
    }
    // SAFETY: LLVM's location is live while its entry is, and this copies from it right away.
    unsafe {
        Some(RemarkDebugLocation {
            source_file_path: remark_string(raw::LLVMRemarkDebugLocGetSourceFilePath(location)),
            line: raw::LLVMRemarkDebugLocGetSourceLine(location),
            column: raw::LLVMRemarkDebugLocGetSourceColumn(location),
        })
    }
}

/// One argument of a remark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemarkArgument {
    /// Its key.
    pub key: String,
    /// Its value.
    pub value: String,
    /// Where it is about, in the source.
    pub debug_location: Option<RemarkDebugLocation>,
}

/// One remark.
pub struct RemarkEntry<'parser> {
    raw: raw::LLVMRemarkEntryRef,
    marker: PhantomData<&'parser ()>,
}

impl Drop for RemarkEntry<'_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the entry.
        unsafe { raw::LLVMRemarkEntryDispose(self.raw) };
    }
}

impl RemarkEntry<'_> {
    /// What kind of remark this is.
    #[must_use]
    pub fn remark_type(&self) -> RemarkType {
        use raw::LLVMRemarkType as T;
        // SAFETY: the entry is live.
        match unsafe { raw::LLVMRemarkEntryGetType(self.raw) } {
            T::LLVMRemarkTypePassed => RemarkType::Passed,
            T::LLVMRemarkTypeMissed => RemarkType::Missed,
            T::LLVMRemarkTypeAnalysis => RemarkType::Analysis,
            T::LLVMRemarkTypeAnalysisFPCommute => RemarkType::AnalysisFloatCommute,
            T::LLVMRemarkTypeAnalysisAliasing => RemarkType::AnalysisAliasing,
            T::LLVMRemarkTypeFailure => RemarkType::Failure,
            _ => RemarkType::Unknown,
        }
    }

    /// The pass that made it.
    #[must_use]
    pub fn pass_name(&self) -> String {
        // SAFETY: the entry is live.
        remark_string(unsafe { raw::LLVMRemarkEntryGetPassName(self.raw) })
    }

    /// Its name.
    #[must_use]
    pub fn remark_name(&self) -> String {
        // SAFETY: the entry is live.
        remark_string(unsafe { raw::LLVMRemarkEntryGetRemarkName(self.raw) })
    }

    /// The function it is about.
    #[must_use]
    pub fn function_name(&self) -> String {
        // SAFETY: the entry is live.
        remark_string(unsafe { raw::LLVMRemarkEntryGetFunctionName(self.raw) })
    }

    /// Where it is about, in the source.
    #[must_use]
    pub fn debug_location(&self) -> Option<RemarkDebugLocation> {
        // SAFETY: the entry is live.
        debug_location(unsafe { raw::LLVMRemarkEntryGetDebugLoc(self.raw) })
    }

    /// How hot this code is, by profile, zero if unknown.
    #[must_use]
    pub fn hotness(&self) -> u64 {
        // SAFETY: the entry is live.
        unsafe { raw::LLVMRemarkEntryGetHotness(self.raw) }
    }

    /// How many arguments it has.
    #[must_use]
    pub fn argument_count(&self) -> u32 {
        // SAFETY: the entry is live.
        unsafe { raw::LLVMRemarkEntryGetNumArgs(self.raw) }
    }

    /// Its arguments.
    #[must_use]
    pub fn arguments(&self) -> Vec<RemarkArgument> {
        let mut arguments = Vec::with_capacity(self.argument_count() as usize);
        // SAFETY: the entry is live, as are its arguments, and each next one comes from the last until a null.
        unsafe {
            let mut next = raw::LLVMRemarkEntryGetFirstArg(self.raw);
            while !next.is_null() {
                arguments.push(RemarkArgument {
                    key: remark_string(raw::LLVMRemarkArgGetKey(next)),
                    value: remark_string(raw::LLVMRemarkArgGetValue(next)),
                    debug_location: debug_location(raw::LLVMRemarkArgGetDebugLoc(next)),
                });
                next = raw::LLVMRemarkEntryGetNextArg(next, self.raw);
            }
        }
        arguments
    }
}

/// Reads remarks from bytes, which it borrows.
pub struct RemarkParser<'data> {
    raw: raw::LLVMRemarkParserRef,
    marker: PhantomData<&'data [u8]>,
}

impl Drop for RemarkParser<'_> {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the parser.
        unsafe { raw::LLVMRemarkParserDispose(self.raw) };
    }
}

impl<'data> RemarkParser<'data> {
    /// Reads the YAML format.
    #[must_use]
    pub fn yaml(data: &'data [u8]) -> Self {
        // SAFETY: the bytes last for `'data`, as long as the parser.
        RemarkParser {
            raw: unsafe { raw::LLVMRemarkParserCreateYAML(data.as_ptr().cast(), data.len() as u64) },
            marker: PhantomData,
        }
    }

    /// Reads the bitstream format.
    #[must_use]
    pub fn bitstream(data: &'data [u8]) -> Self {
        // SAFETY: the bytes last for `'data`, as long as the parser.
        RemarkParser {
            raw: unsafe { raw::LLVMRemarkParserCreateBitstream(data.as_ptr().cast(), data.len() as u64) },
            marker: PhantomData,
        }
    }

    /// The next remark, or none at the end.
    ///
    /// # Errors
    /// What is wrong with the data.
    pub fn next_remark(&mut self) -> Result<Option<RemarkEntry<'_>>, Error> {
        // SAFETY: the parser is live; a null next with an error set means the data was bad, and the message is copied.
        unsafe {
            let next = raw::LLVMRemarkParserGetNext(self.raw);
            if next.is_null() {
                if raw::LLVMRemarkParserHasError(self.raw) != 0 {
                    return Err(Error::new(from_borrowed(raw::LLVMRemarkParserGetErrorMessage(self.raw))));
                }
                return Ok(None);
            }
            Ok(Some(RemarkEntry { raw: next, marker: PhantomData }))
        }
    }
}
