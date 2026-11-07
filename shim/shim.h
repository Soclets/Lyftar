// What the shim's source files share.

#ifndef LYFTAR_SHIM_H
#define LYFTAR_SHIM_H

#include "llvm/IR/Function.h"
#include "llvm/Support/ErrorHandling.h"

#include <string>

namespace lyftar {

// A broken contract is a bug in Lyftar itself. The shim is built with
// -fno-exceptions, so it stops here. trace: D25
inline void require(bool condition, const char *message) {
  if (!condition) {
    llvm::report_fatal_error(message, false);
  }
}

// A report of what would make LLVM end the process when it looks up this
// function's garbage collection strategy, or an empty string. trace: D45
std::string strategyProblem(const llvm::Function &function);

} // namespace lyftar

#endif
