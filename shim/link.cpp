// lld through its library (D49, D52). lld has no C interface, so this is the
// shim's way in. Each function's contract says who owns each pointer; unless it
// says otherwise, every pointer is live, non-null and borrowed for the call.
// Rust runs one link at a time, since two at once crash lld. trace: D52

#include "shim.h"

#include "llvm-c/Core.h"
#include "llvm/ADT/ArrayRef.h"
#include "llvm/Object/Archive.h"
#include "llvm/Object/Binary.h"
#include "llvm/Object/ObjectFile.h"
#include "llvm/Support/Error.h"
#include "llvm/Support/MemoryBufferRef.h"
#include "llvm/Support/raw_ostream.h"

#include "lld/Common/Driver.h"

#include <array>
#include <cstddef>
#include <cstdint>
#include <memory>
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
  const std::array<lld::DriverDef, 2> drivers = {
      lld::DriverDef{.f = lld::Gnu, .d = &lld::elf::link},
      lld::DriverDef{.f = lld::Darwin, .d = &lld::macho::link},
  };
  const lld::Result result =
      lld::lldMain(llvm::ArrayRef<const char *>(arguments, count),
                   standard_output, standard_error, drivers);
  standard_output.flush();
  standard_error.flush();
  *output = LLVMCreateMessage(printed.c_str());
  *can_run_again = result.canRunAgain;
  return result.retCode;
}

namespace {

// Adds the global symbols one object defines and leaves undefined to the two
// lists, one name per line.
void collectSymbols(const llvm::object::ObjectFile &object,
                    std::string &defined, std::string &undefined) {
  for (const llvm::object::SymbolRef &symbol : object.symbols()) {
    llvm::Expected<uint32_t> flags = symbol.getFlags();
    llvm::Expected<llvm::StringRef> name = symbol.getName();
    if (!flags || !name) {
      llvm::consumeError(flags.takeError());
      llvm::consumeError(name.takeError());
      continue;
    }
    if ((*flags & llvm::object::SymbolRef::SF_Global) == 0 &&
        (*flags & llvm::object::SymbolRef::SF_Undefined) == 0) {
      continue;
    }
    std::string &into = (*flags & llvm::object::SymbolRef::SF_Undefined) != 0
                            ? undefined
                            : defined;
    into.append(name->data(), name->size());
    into.push_back('\n');
  }
}

} // namespace

// Contract: `bytes` is readable for `size` bytes: an object file, or an archive
// of them. `defined` and `undefined` are written with the global symbols it
// defines and leaves undefined, one name per line, for the caller to free with
// `LLVMDisposeMessage`. Returns false, writing neither, if it can't be read.
extern "C" bool lyftar_object_symbols(const char *bytes, size_t size,
                                      char **defined, char **undefined) {
  require(bytes != nullptr && defined != nullptr && undefined != nullptr,
          "lyftar_object_symbols: null");
  const llvm::MemoryBufferRef buffer(llvm::StringRef(bytes, size), "object");
  llvm::Expected<std::unique_ptr<llvm::object::Binary>> binary =
      llvm::object::createBinary(buffer);
  if (!binary) {
    llvm::consumeError(binary.takeError());
    return false;
  }
  std::string defined_names;
  std::string undefined_names;
  if (const auto *object =
          llvm::dyn_cast<llvm::object::ObjectFile>(binary->get())) {
    collectSymbols(*object, defined_names, undefined_names);
  } else if (const auto *archive =
                 llvm::dyn_cast<llvm::object::Archive>(binary->get())) {
    llvm::Error failure = llvm::Error::success();
    for (const llvm::object::Archive::Child &child :
         archive->children(failure)) {
      llvm::Expected<std::unique_ptr<llvm::object::Binary>> member =
          child.getAsBinary();
      if (!member) {
        llvm::consumeError(member.takeError());
        continue;
      }
      if (const auto *object =
              llvm::dyn_cast<llvm::object::ObjectFile>(member->get())) {
        collectSymbols(*object, defined_names, undefined_names);
      }
    }
    if (failure) {
      llvm::consumeError(std::move(failure));
      return false;
    }
  } else {
    return false;
  }
  *defined = LLVMCreateMessage(defined_names.c_str());
  *undefined = LLVMCreateMessage(undefined_names.c_str());
  return true;
}
