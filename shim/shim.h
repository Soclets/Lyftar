// What the shim's source files share.

#ifndef LYFTAR_SHIM_H
#define LYFTAR_SHIM_H

#include "llvm/Support/ErrorHandling.h"

namespace lyftar {

// A broken contract is a bug in Lyftar itself. The shim is built with
// -fno-exceptions, so it stops here. trace: D25
inline void require(bool condition, const char *message) {
  if (!condition) {
    llvm::report_fatal_error(message, false);
  }
}

} // namespace lyftar

#endif
