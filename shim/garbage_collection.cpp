// Garbage collection (D45): what LLVM's C API has nothing for, which is the
// registry of strategies. Each function's contract says who owns each pointer;
// unless it says otherwise, every pointer is live, non-null and borrowed for
// the call. trace: D45

#include "shim.h"

#include "llvm-c/Core.h"
#include "llvm-c/Types.h"
#include "llvm/ADT/STLExtras.h"
#include "llvm/ADT/StringRef.h"
#include "llvm/IR/BasicBlock.h"
#include "llvm/IR/BuiltinGCs.h"
#include "llvm/IR/Constants.h"
#include "llvm/IR/DerivedTypes.h"
#include "llvm/IR/Function.h"
#include "llvm/IR/GCStrategy.h"
#include "llvm/IR/Instructions.h"
#include "llvm/IR/IntrinsicInst.h"
#include "llvm/IR/Module.h"
#include "llvm/IR/Statepoint.h"
#include "llvm/IR/Type.h"
#include "llvm/Support/CBindingWrapping.h"
#include "llvm/Support/raw_ostream.h"
#include "llvm/TargetParser/Triple.h"

#include <cstddef>
#include <string>

using namespace llvm;
using lyftar::require;

namespace {

// `getGCStrategy` ends the process on a name nothing registered, so everything
// asks this first.
bool isRegistered(StringRef name) {
  linkAllBuiltinGCs();
  return any_of(GCRegistry::entries(), [name](const GCRegistry::entry &entry) {
    return entry.getName() == name;
  });
}

} // namespace

// Contract: `name` is readable for `length` bytes, and need not end in NUL.
// Returns whether a strategy of this name is registered.
extern "C" bool lyftar_strategy_is_registered(const char *name, size_t length) {
  require(name != nullptr || length == 0,
          "lyftar_strategy_is_registered: null name");
  return isRegistered(StringRef(name, length));
}

// Contract: `function` is a live function with a registered garbage collector,
// and `type` is a live pointer type. Returns 1 if its strategy manages pointers
// of this type, 0 if it doesn't, and -1 if it can't tell.
extern "C" int lyftar_strategy_manages_pointer(LLVMValueRef function,
                                               LLVMTypeRef type) {
  require(function != nullptr && isa<Function>(unwrap(function)),
          "lyftar_strategy_manages_pointer: not a function");
  const auto *found = cast<Function>(unwrap(function));
  require(found->hasGC() && isRegistered(found->getGC()),
          "lyftar_strategy_manages_pointer: no registered garbage collector");
  require(type != nullptr && isa<PointerType>(unwrap(type)),
          "lyftar_strategy_manages_pointer: not a pointer type");
  const std::optional<bool> managed =
      getGCStrategy(found->getGC())->isGCManagedPointer(unwrap(type));
  if (!managed) {
    return -1;
  }
  return *managed ? 1 : 0;
}

std::string lyftar::strategyProblem(const Function &function) {
  std::string report;
  raw_string_ostream stream(report);
  if (function.hasGC() && !isRegistered(function.getGC())) {
    stream << "function '" << function.getName()
           << "' uses a garbage collector nothing registered: '"
           << function.getGC() << "'";
    return report;
  }
  if (function.hasGC()) {
    return report;
  }
  // The verifier looks up the strategy of any function with a `gc.relocate` in
  // it, whether it has one or not.
  for (const BasicBlock &block : function) {
    for (const Instruction &instruction : block) {
      if (isa<GCRelocateInst>(instruction)) {
        stream << "function '" << function.getName()
               << "' has a gc.relocate and no garbage collector";
        return report;
      }
    }
  }
  return report;
}

// Contract: `module` is live. Returns null if LLVM's verifier and passes can
// look up every strategy this module needs, and otherwise a report for the
// caller to free, since they end the process over one they can't find.
extern "C" char *lyftar_module_strategy_problem(LLVMModuleRef module) {
  require(module != nullptr, "lyftar_module_strategy_problem: null module");
  for (const Function &function : *unwrap(module)) {
    const std::string report = lyftar::strategyProblem(function);
    if (!report.empty()) {
      return LLVMCreateMessage(report.c_str());
    }
  }
  return nullptr;
}

// Contract: `module` is live. Returns whether `place-safepoints` would need a
// `gc.safepoint_poll` this module doesn't define. That pass only rewrites these
// two strategies, and in a release build of LLVM reads a null function
// otherwise.
extern "C" bool lyftar_module_needs_safepoint_poll(LLVMModuleRef module) {
  require(module != nullptr, "lyftar_module_needs_safepoint_poll: null module");
  const Module *found = unwrap(module);
  const Function *poll = found->getFunction("gc.safepoint_poll");
  if (poll != nullptr && !poll->isDeclaration() && poll->arg_empty() &&
      poll->getReturnType()->isVoidTy()) {
    return false;
  }
  return any_of(*found, [](const Function &function) {
    return !function.isDeclaration() && function.hasGC() &&
           (function.getGC() == "statepoint-example" ||
            function.getGC() == "coreclr");
  });
}

// Contract: `value` is live. Returns whether, past any pointer casts, it is a
// fixed-size stack allocation in its function's entry block, the only kind that
// gets a stack slot of its own for `llvm.gcroot` to name.
extern "C" bool lyftar_value_is_static_stack_allocation(LLVMValueRef value) {
  require(value != nullptr, "lyftar_value_is_static_stack_allocation: null");
  const auto *allocation =
      dyn_cast<AllocaInst>(unwrap(value)->stripPointerCasts());
  return allocation != nullptr && allocation->isStaticAlloca();
}

// Contract: `value` is live. Returns whether it is a stack allocation, past any
// pointer casts.
extern "C" bool lyftar_value_is_stack_allocation(LLVMValueRef value) {
  require(value != nullptr, "lyftar_value_is_stack_allocation: null");
  return isa<AllocaInst>(unwrap(value)->stripPointerCasts());
}

// Contract: `value` is a stack allocation, past any pointer casts. Returns
// whether what it holds is a pointer.
extern "C" bool lyftar_stack_allocation_holds_a_pointer(LLVMValueRef value) {
  require(value != nullptr, "lyftar_stack_allocation_holds_a_pointer: null");
  const auto *allocation =
      dyn_cast<AllocaInst>(unwrap(value)->stripPointerCasts());
  require(allocation != nullptr,
          "lyftar_stack_allocation_holds_a_pointer: not a stack allocation");
  return allocation->getAllocatedType()->isPointerTy();
}

// Contract: `value` is live. Returns whether it calls or invokes
// `gc.statepoint`.
extern "C" bool lyftar_value_is_statepoint(LLVMValueRef value) {
  require(value != nullptr, "lyftar_value_is_statepoint: null");
  return isa<GCStatepointInst>(unwrap(value));
}

// Contract: `block` is live. Returns its one predecessor, or null if it doesn't
// have exactly one.
extern "C" LLVMBasicBlockRef
lyftar_block_unique_predecessor(LLVMBasicBlockRef block) {
  require(block != nullptr, "lyftar_block_unique_predecessor: null");
  return wrap(unwrap(block)->getUniquePredecessor());
}

// Contract: `module` is live. Returns whether a statepoint in it has the
// transition flag. Only x86 can make code for one, and every other target ends
// the process.
extern "C" bool lyftar_module_has_transition_statepoint(LLVMModuleRef module) {
  require(module != nullptr,
          "lyftar_module_has_transition_statepoint: null module");
  const auto hasTransition = [](const User *user) {
    const auto *statepoint = dyn_cast<GCStatepointInst>(user);
    return statepoint != nullptr &&
           (statepoint->getFlags() &
            static_cast<uint64_t>(StatepointFlags::GCTransition)) != 0;
  };
  return any_of(*unwrap(module), [&hasTransition](const Function &function) {
    return function.getIntrinsicID() == Intrinsic::experimental_gc_statepoint &&
           any_of(function.users(), hasTransition);
  });
}

// Contract: `module` is live, and `triple` is a C string. Returns whether a
// function in it uses `erlang` while this triple doesn't make ELF. That
// strategy's table printer always writes an ELF section, which corrupts memory
// in any other format's streamer.
extern "C" bool lyftar_module_needs_elf_for_its_tables(LLVMModuleRef module,
                                                       const char *triple) {
  require(module != nullptr && triple != nullptr,
          "lyftar_module_needs_elf_for_its_tables: null");
  return !Triple(triple).isOSBinFormatELF() &&
         any_of(*unwrap(module), [](const Function &function) {
           return function.hasGC() && function.getGC() == "erlang";
         });
}
