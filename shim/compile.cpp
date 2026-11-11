// clang through its library, to compile C to an object (D55). clang has no C
// interface for this, so this is the shim's way in. Each function's contract
// says who owns each pointer; unless it says otherwise, every pointer is live,
// non-null and borrowed for the call. Each call has a compiler instance of its
// own, so calls can run at once. trace: D55

#include "shim.h"

#include "llvm-c/Core.h"
#include "llvm/ADT/ArrayRef.h"
#include "llvm/ADT/IntrusiveRefCntPtr.h"
#include "llvm/Support/VirtualFileSystem.h"
#include "llvm/Support/raw_ostream.h"

#include "clang/Basic/Diagnostic.h"
#include "clang/Basic/DiagnosticOptions.h"
#include "clang/CodeGen/CodeGenAction.h"
#include "clang/Driver/CreateInvocationFromArgs.h"
#include "clang/Frontend/CompilerInstance.h"
#include "clang/Frontend/CompilerInvocation.h"
#include "clang/Frontend/TextDiagnosticPrinter.h"

#include <cstddef>
#include <memory>
#include <string>
#include <vector>

using lyftar::require;

// Contract: `arguments` holds `count` C strings: clang's driver arguments
// without the program's name, `-c` or `-o`, naming one C source file. `output`
// is the object's path. The caller has set up every target. `messages` is
// written with clang's diagnostics, for the caller to free with
// `LLVMDisposeMessage`. Returns whether the object was written.
extern "C" bool lyftar_compile_c(const char *const *arguments, size_t count,
                                 const char *output, char **messages) {
  require(arguments != nullptr && count > 0 && output != nullptr &&
              messages != nullptr,
          "lyftar_compile_c: null");
  std::string printed;
  llvm::raw_string_ostream stream(printed);
  clang::DiagnosticOptions options;
  clang::TextDiagnosticPrinter printer(stream, options);
  const llvm::IntrusiveRefCntPtr<llvm::vfs::FileSystem> files =
      llvm::vfs::getRealFileSystem();
  // The driver's own name comes first, as in a command.
  std::vector<const char *> driver{"clang"};
  const llvm::ArrayRef<const char *> given(arguments, count);
  driver.insert(driver.end(), given.begin(), given.end());
  clang::CreateInvocationOptions create;
  create.Diags = clang::CompilerInstance::createDiagnostics(
      *files, options, &printer, /*ShouldOwnClient=*/false);
  create.VFS = files;
  std::shared_ptr<clang::CompilerInvocation> invocation =
      clang::createInvocation(driver, create);
  bool written = false;
  if (invocation != nullptr) {
    // The driver asks a separate process to skip freeing at exit, which in a
    // library would leak every compile.
    invocation->getFrontendOpts().DisableFree = false;
    invocation->getCodeGenOpts().DisableFree = false;
    // `createInvocation` only checks syntax, so the object is asked for here.
    invocation->getFrontendOpts().OutputFile = output;
    clang::CompilerInstance instance(std::move(invocation));
    instance.createDiagnostics(&printer, /*ShouldOwnClient=*/false);
    clang::EmitObjAction action;
    written = instance.ExecuteAction(action);
  }
  stream.flush();
  *messages = LLVMCreateMessage(printed.c_str());
  return written;
}
