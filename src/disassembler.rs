//! Disassembling machine code into text.

use crate::raw;
use crate::text::c_string;

/// A disassembler for one target.
pub struct Disassembler {
    raw: raw::LLVMDisasmContextRef,
}

impl Drop for Disassembler {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns it.
        unsafe { raw::LLVMDisasmDispose(self.raw) };
    }
}

/// Options for [`Disassembler::set_options`], which can be combined with `|`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisassemblerOptions(u64);

impl DisassemblerOptions {
    /// Prints immediates in hexadecimal.
    pub const PRINT_IMMEDIATES_AS_HEX: Self = DisassemblerOptions(raw::LLVMDisassembler_Option_PrintImmHex as u64);
    /// Prints with the alternate syntax, if the target has one.
    pub const ALTERNATE_SYNTAX: Self = DisassemblerOptions(raw::LLVMDisassembler_Option_AsmPrinterVariant as u64);
    /// Puts comments in the text.
    pub const SET_INSTRUCTION_COMMENTS: Self =
        DisassemblerOptions(raw::LLVMDisassembler_Option_SetInstrComments as u64);
    /// Prints the latency.
    pub const PRINT_LATENCY: Self = DisassemblerOptions(raw::LLVMDisassembler_Option_PrintLatency as u64);
    /// Prints the encoding.
    pub const COLOR: Self = DisassemblerOptions(raw::LLVMDisassembler_Option_Color as u64);
}

impl std::ops::BitOr for DisassemblerOptions {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        DisassemblerOptions(self.0 | other.0)
    }
}

impl Disassembler {
    /// A disassembler for `triple`, with this processor and these features, which can be empty. It gives none if LLVM has no
    /// disassembler for that target.
    ///
    /// # Panics
    /// If a string has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn new(triple: &str, cpu: &str, features: &str) -> Option<Self> {
        let triple = c_string(triple, "Disassembler::new");
        let cpu = c_string(cpu, "Disassembler::new");
        let features = c_string(features, "Disassembler::new");
        // SAFETY: the strings are valid C strings for the call; no callbacks and no state, so symbols aren't looked up.
        let raw_context = unsafe {
            raw::LLVMCreateDisasmCPUFeatures(
                triple.as_ptr(),
                cpu.as_ptr(),
                features.as_ptr(),
                std::ptr::null_mut(),
                0,
                None,
                None,
            )
        };
        (!raw_context.is_null()).then_some(Disassembler { raw: raw_context })
    }

    /// Sets options. It says whether LLVM took them.
    pub fn set_options(&mut self, options: DisassemblerOptions) -> bool {
        // SAFETY: the disassembler is live.
        unsafe { raw::LLVMSetDisasmOptions(self.raw, options.0) != 0 }
    }

    /// The instruction at the start of `bytes`, whose address is `address`: its size and its text. It gives none if the bytes
    /// aren't an instruction.
    #[must_use]
    pub fn disassemble_instruction(&mut self, bytes: &[u8], address: u64) -> Option<(usize, String)> {
        let mut text = [0_u8; 256];
        // SAFETY: the disassembler is live, and LLVM only reads the bytes and writes a NUL-terminated string that fits. Its
        // signature wants a mutable pointer by old habit.
        let size = unsafe {
            raw::LLVMDisasmInstruction(
                self.raw,
                bytes.as_ptr().cast_mut(),
                bytes.len() as u64,
                address,
                text.as_mut_ptr().cast(),
                text.len(),
            )
        };
        if size == 0 {
            return None;
        }
        let end = text.iter().position(|byte| *byte == 0).unwrap_or(text.len());
        Some((size, String::from_utf8_lossy(&text[..end]).into_owned()))
    }
}
