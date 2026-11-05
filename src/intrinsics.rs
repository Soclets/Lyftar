//! LLVM's intrinsic functions, looked up by name.

use crate::bad_call::bad_call;
use crate::context::Context;
use crate::raw;
use crate::text::from_pointer_and_length;
use crate::types::Type;

/// An intrinsic, from [`Intrinsic::find`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Intrinsic(pub(crate) u32);

impl Intrinsic {
    /// The intrinsic with this name, such as `llvm.sadd.with.overflow`, if there is one.
    #[must_use]
    pub fn find(name: &str) -> Option<Intrinsic> {
        // SAFETY: LLVM reads `name.len()` bytes.
        let id = unsafe { raw::LLVMLookupIntrinsicID(name.as_ptr().cast(), name.len()) };
        (id != 0).then_some(Intrinsic(id))
    }

    /// Whether this intrinsic is overloaded on the types it works with.
    #[must_use]
    pub fn is_overloaded(self) -> bool {
        // SAFETY: an `Intrinsic` only holds an id LLVM gave back.
        unsafe { raw::LLVMIntrinsicIsOverloaded(self.0) != 0 }
    }

    /// This intrinsic's base name.
    #[must_use]
    pub fn name(self) -> String {
        let mut length = 0;
        // SAFETY: an `Intrinsic` only holds an id LLVM gave back; the text is copied right away.
        let text = unsafe { raw::LLVMIntrinsicGetName(self.0, &raw mut length) };
        from_pointer_and_length(text, length)
    }

    /// This intrinsic's function type for the given overload types.
    ///
    /// # Panics
    /// If it isn't overloaded on that many types (D3).
    #[must_use]
    #[track_caller]
    pub fn function_type<'context>(
        self,
        context: &Context<'context>,
        overload_types: &[Type<'context>],
    ) -> Type<'context> {
        self.check_overloads("function_type", overload_types);
        // SAFETY: the overloads were just checked and LLVM only reads them.
        unsafe {
            Type::from_raw(raw::LLVMIntrinsicGetType(
                context.raw,
                self.0,
                overload_types.as_ptr().cast_mut().cast(),
                overload_types.len(),
            ))
        }
    }

    #[track_caller]
    pub(crate) fn check_overloads(self, operation: &str, overload_types: &[Type<'_>]) {
        // SAFETY: only reads the id and types.
        let matches = unsafe {
            crate::shim::lyftar_intrinsic_overloads_match(self.0, overload_types.as_ptr().cast(), overload_types.len())
        };
        if !matches {
            bad_call(format_args!(
                "{operation}: {} isn't overloaded on {} types like those",
                self.name(),
                overload_types.len()
            ));
        }
    }
}
