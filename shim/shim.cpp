// What the generated bindings cannot reach (D16). Each function's contract says who owns each pointer, how long it
// lives and which values are allowed; each function checks it at the top, and Rust wraps each one once, in a safe
// function that checks it too (rule 9, D25).

#include "llvm-c/Core.h"
#include "llvm-c/Target.h"
#include "llvm-c/Types.h"
#include "llvm/ADT/SmallPtrSet.h"
#include "llvm/ADT/SmallVector.h"
#include "llvm/IR/BasicBlock.h"
#include "llvm/IR/Constants.h"
#include "llvm/IR/Function.h"
#include "llvm/IR/IRBuilder.h"
#include "llvm/IR/Instruction.h"
#include "llvm/IR/Module.h"
#include "llvm/Support/ErrorHandling.h"

using namespace llvm;

namespace {

// A broken contract is a bug in Lyftar itself. The shim is built with -fno-exceptions, so it stops here.
// trace: D25
void require(bool condition, const char *message) {
  if (!condition) {
    report_fatal_error(message, false);
  }
}

} // namespace

// Contract: `LLVMInitializeNative*` are inline functions in LLVM's C header, so they are compiled here. Takes nothing
// and returns 0 on success. Safe to call more than once.
extern "C" int lyftar_initialize_native_target() {
  return static_cast<int>(LLVMInitializeNativeTarget() != 0 ||
                          LLVMInitializeNativeAsmPrinter() != 0 ||
                          LLVMInitializeNativeAsmParser() != 0);
}

// Contract: `value` and `module` are live and non-null and borrowed for the call. Returns whether `value` can be used
// in `module`: an instruction, argument, block or global must be in it, and a constant must only reach globals in it.
// A constant with no operands belongs to the context, so it can be used anywhere.
// trace: D11
extern "C" bool lyftar_value_belongs_to_module(LLVMValueRef value,
                                               LLVMModuleRef module) {
  require(value != nullptr && module != nullptr,
          "lyftar_value_belongs_to_module: null argument");
  const Value *start = unwrap(value);
  const Module *wanted = unwrap(module);
  if (const auto *instruction = dyn_cast<Instruction>(start)) {
    return instruction->getModule() == wanted;
  }
  if (const auto *argument = dyn_cast<Argument>(start)) {
    return argument->getParent()->getParent() == wanted;
  }
  if (const auto *block = dyn_cast<BasicBlock>(start)) {
    return block->getModule() == wanted;
  }
  if (isa<ConstantData>(start)) {
    return true;
  }
  if (const auto *global = dyn_cast<GlobalValue>(start)) {
    return global->getParent() == wanted;
  }
  const auto *constant = dyn_cast<Constant>(start);
  if (constant == nullptr) {
    // Inline assembly and metadata wrapped as a value belong to the context.
    return true;
  }
  // A constant expression or aggregate belongs wherever the globals it reaches do.
  SmallVector<const Constant *, 8> work{constant};
  SmallPtrSet<const Constant *, 8> seen;
  while (!work.empty()) {
    const Constant *current = work.pop_back_val();
    if (!seen.insert(current).second || isa<ConstantData>(current)) {
      continue;
    }
    if (const auto *global = dyn_cast<GlobalValue>(current)) {
      if (global->getParent() != wanted) {
        return false;
      }
      continue;
    }
    for (const Use &operand : current->operands()) {
      if (const auto *inner = dyn_cast<Constant>(operand.get())) {
        work.push_back(inner);
      }
    }
  }
  return true;
}

// Contract: `builder` is live, non-null, placed in a block, and borrowed for the call. Returns whether an instruction
// can still go in there: false when placed at the end of a block that already ends in a terminator.
// trace: D3
extern "C" bool lyftar_builder_is_open(LLVMBuilderRef builder) {
  require(builder != nullptr, "lyftar_builder_is_open: null builder");
  const IRBuilder<> *placed = unwrap(builder);
  const BasicBlock *block = placed->GetInsertBlock();
  require(block != nullptr, "lyftar_builder_is_open: builder has no position");
  return placed->GetInsertPoint() != block->end() ||
         !block->hasTerminator();
}
