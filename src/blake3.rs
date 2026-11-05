//! The BLAKE3 hash, as LLVM carries it.

use crate::raw;
use crate::text::from_borrowed;

/// How many bytes a BLAKE3 key has.
pub const KEY_SIZE_IN_BYTES: usize = raw::LLVM_BLAKE3_KEY_LEN as usize;
/// How many bytes a BLAKE3 hash has by default.
pub const OUTPUT_SIZE_IN_BYTES: usize = raw::LLVM_BLAKE3_OUT_LEN as usize;

/// A hash in progress.
pub struct Blake3Hasher {
    raw: raw::llvm_blake3_hasher,
}

/// LLVM's version of BLAKE3.
#[must_use]
pub fn version() -> String {
    // SAFETY: takes nothing; the text is copied right away.
    from_borrowed(unsafe { raw::llvm_blake3_version() })
}

impl Blake3Hasher {
    fn initialized(initialize: impl FnOnce(*mut raw::llvm_blake3_hasher)) -> Self {
        let mut hasher = std::mem::MaybeUninit::<raw::llvm_blake3_hasher>::uninit();
        initialize(hasher.as_mut_ptr());
        // SAFETY: every initializer passed here sets up the whole struct.
        Blake3Hasher { raw: unsafe { hasher.assume_init() } }
    }

    /// A plain hash.
    #[must_use]
    pub fn new() -> Self {
        // SAFETY: the pointer has room for the hasher, which this fills.
        Self::initialized(|hasher| unsafe { raw::llvm_blake3_hasher_init(hasher) })
    }

    /// A keyed hash.
    #[must_use]
    pub fn new_keyed(key: &[u8; KEY_SIZE_IN_BYTES]) -> Self {
        // SAFETY: the pointer has room for the hasher, which this fills, and the key has the size LLVM reads.
        Self::initialized(|hasher| unsafe { raw::llvm_blake3_hasher_init_keyed(hasher, key.as_ptr()) })
    }

    /// A key derivation, for this application's context bytes.
    #[must_use]
    pub fn new_derive_key(context: &[u8]) -> Self {
        // SAFETY: the pointer has room for the hasher, which this fills, and LLVM reads the length given.
        Self::initialized(|hasher| unsafe {
            raw::llvm_blake3_hasher_init_derive_key_raw(hasher, context.as_ptr().cast(), context.len());
        })
    }

    /// A key derivation, for this application's context text, which LLVM reads up to a NUL.
    ///
    /// # Panics
    /// If `context` has a NUL byte inside it (D3).
    #[must_use]
    #[track_caller]
    pub fn new_derive_key_from_text(context: &str) -> Self {
        let context = crate::text::c_string(context, "new_derive_key_from_text");
        // SAFETY: the pointer has room for the hasher, which this fills, and the context is a valid C string for the call.
        Self::initialized(|hasher| unsafe { raw::llvm_blake3_hasher_init_derive_key(hasher, context.as_ptr()) })
    }

    /// Adds bytes to what is hashed.
    pub fn update(&mut self, input: &[u8]) {
        // SAFETY: the hasher is set up, and LLVM reads the length given.
        unsafe { raw::llvm_blake3_hasher_update(&raw mut self.raw, input.as_ptr().cast(), input.len()) };
    }

    /// The hash so far. It can go on being updated.
    #[must_use]
    pub fn finalize(&self) -> [u8; OUTPUT_SIZE_IN_BYTES] {
        let mut output = [0; OUTPUT_SIZE_IN_BYTES];
        self.finalize_into(&mut output);
        output
    }

    /// The hash so far, as long as `output` asks.
    pub fn finalize_into(&self, output: &mut [u8]) {
        // SAFETY: the hasher is set up, and LLVM writes the length given.
        unsafe { raw::llvm_blake3_hasher_finalize(&raw const self.raw, output.as_mut_ptr(), output.len()) };
    }

    /// The hash's output from `seek_in_bytes` on, as long as `output` asks.
    pub fn finalize_at(&self, seek_in_bytes: u64, output: &mut [u8]) {
        // SAFETY: the hasher is set up, and LLVM writes the length given.
        unsafe {
            raw::llvm_blake3_hasher_finalize_seek(&raw const self.raw, seek_in_bytes, output.as_mut_ptr(), output.len())
        };
    }

    /// Starts over, keeping the key or context.
    pub fn reset(&mut self) {
        // SAFETY: the hasher is set up.
        unsafe { raw::llvm_blake3_hasher_reset(&raw mut self.raw) };
    }
}

impl Default for Blake3Hasher {
    fn default() -> Self {
        Self::new()
    }
}
