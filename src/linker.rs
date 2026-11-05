//! Linking modules together.

use crate::error::Error;
use crate::module::Module;
use crate::shim;

impl<'context> Module<'context> {
    // trace: D11
    /// Links `source` into this module, which destroys `source`. Declarations here may be replaced by definitions there,
    /// so this needs exclusive access, as a delete does.
    ///
    /// # Errors
    /// What the linker reported, such as two definitions of one symbol.
    pub fn link_in(&mut self, source: Module<'context>) -> Result<(), Error> {
        self.bump_generation();
        let mut message = std::ptr::null_mut();
        // SAFETY: both are live in one context, since `'context` is invariant. The call owns `source` from here on, and
        // writes a message for the caller to free if it fails.
        let failed = unsafe { shim::lyftar_link_modules(self.raw, source.into_raw(), &raw mut message) };
        if failed { Err(Error::from_owned_message(message)) } else { Ok(()) }
    }
}
