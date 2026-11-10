// lld through its library (D49, D52). lld has no C interface, so this is the
// shim's way in. Each function's contract says who owns each pointer; unless it
// says otherwise, every pointer is live, non-null and borrowed for the call.
// Rust runs one link at a time, since two at once crash lld. trace: D52

#include "shim.h"

#include "llvm-c/Core.h"
#include "llvm/ADT/ArrayRef.h"
#include "llvm/Support/raw_ostream.h"

#include "lld/Common/Driver.h"

#include <cstddef>
#include <string>

LLD_HAS_DRIVER(elf)
LLD_HAS_DRIVER(macho)

using lyftar::require;

// Contract: `arguments` holds `count` C strings, the first naming the flavour
// as lld's own executables are named (`ld.lld`, `ld64.lld`). `output` and
// `can_run_again` are written: `output` with what lld printed, for the caller
// to free with `LLVMDisposeMessage`. Returns lld's exit code. The caller holds
// the lock that keeps links one at a time.
extern "C" int lyftar_lld(const char *const *arguments, size_t count,
                          char **output, bool *can_run_again) {
  require(arguments != nullptr && count > 0 && output != nullptr &&
              can_run_again != nullptr,
          "lyftar_lld: null");
  std::string printed;
  llvm::raw_string_ostream standard_output(printed);
  llvm::raw_string_ostream standard_error(printed);
  const lld::Result result = lld::lldMain(
      llvm::ArrayRef<const char *>(arguments, count), standard_output,
      standard_error,
      {{lld::Gnu, &lld::elf::link}, {lld::Darwin, &lld::macho::link}});
  standard_output.flush();
  standard_error.flush();
  *output = LLVMCreateMessage(printed.c_str());
  *can_run_again = result.canRunAgain;
  return result.retCode;
}
