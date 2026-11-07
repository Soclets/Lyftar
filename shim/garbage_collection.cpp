// Garbage collection (D45): what LLVM's C API has nothing for, which is the
// registry of strategies. Each function's contract says who owns each pointer;
// unless it says otherwise, every pointer is live, non-null and borrowed for
// the call. trace: D45

#include "shim.h"

#include "llvm-c/Core.h"
#include "llvm-c/ExecutionEngine.h"
#include "llvm-c/LLJIT.h"
#include "llvm-c/Types.h"
#include "llvm/ADT/ArrayRef.h"
#include "llvm/ADT/STLExtras.h"
#include "llvm/ADT/StringRef.h"
#include "llvm/ExecutionEngine/ExecutionEngine.h"
#include "llvm/ExecutionEngine/JITEventListener.h"
#include "llvm/ExecutionEngine/JITLink/JITLink.h"
#include "llvm/ExecutionEngine/JITSymbol.h"
#include "llvm/ExecutionEngine/Orc/LLJIT.h"
#include "llvm/ExecutionEngine/Orc/LinkGraphLinkingLayer.h"
#include "llvm/ExecutionEngine/Orc/ObjectLinkingLayer.h"
#include "llvm/ExecutionEngine/Orc/RTDyldObjectLinkingLayer.h"
#include "llvm/ExecutionEngine/RuntimeDyld.h"
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
#include "llvm/Object/ObjectFile.h"
#include "llvm/Support/CBindingWrapping.h"
#include "llvm/Support/Error.h"
#include "llvm/Support/raw_ostream.h"
#include "llvm/TargetParser/Triple.h"

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <iterator>
#include <map>
#include <memory>
#include <mutex>
#include <string>
#include <utility>
#include <vector>

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

// Stack maps from a JIT. A JIT makes its code in its own memory, and nothing
// there keeps a stack map section unless something asks to, so these ask.

namespace {

// JITLink names a Mach-O section with its segment.
constexpr StringLiteral MachOStackMaps("__LLVM_STACKMAPS,__llvm_stackmaps");
constexpr StringLiteral ElfStackMaps(".llvm_stackmaps");

bool isStackMapSectionName(StringRef name) {
  return name == "__llvm_stackmaps" || name == ElfStackMaps;
}

// The sections of what a JIT has linked. JITLink's are copied when they are
// relocated, by what removes them, and RuntimeDyld's are read when asked for,
// from memory that lasts until it says it goes.
class CapturedStackMaps final : public JITEventListener {
public:
  void addCopied(orc::ResourceKey key, std::vector<std::string> sections) {
    const std::scoped_lock lock(mutex);
    auto &found = copied[key];
    std::ranges::move(sections, std::back_inserter(found));
  }

  void removeCopied(orc::ResourceKey key) {
    const std::scoped_lock lock(mutex);
    copied.erase(key);
  }

  void transferCopied(orc::ResourceKey destination, orc::ResourceKey source) {
    const std::scoped_lock lock(mutex);
    const auto found = copied.find(source);
    if (found == copied.end()) {
      return;
    }
    auto &into = copied[destination];
    std::ranges::move(found->second, std::back_inserter(into));
    copied.erase(source);
  }

  void notifyObjectLoaded(ObjectKey key, const object::ObjectFile &object,
                          const RuntimeDyld::LoadedObjectInfo &info) override {
    const std::scoped_lock lock(mutex);
    for (const object::SectionRef &section : object.sections()) {
      Expected<StringRef> name = section.getName();
      if (!name) {
        consumeError(name.takeError());
        continue;
      }
      if (!isStackMapSectionName(*name)) {
        continue;
      }
      const uint64_t address = info.getSectionLoadAddress(section);
      if (address != 0) {
        loaded[key].emplace_back(address, section.getSize());
      }
    }
  }

  void notifyFreeingObject(ObjectKey key) override {
    const std::scoped_lock lock(mutex);
    loaded.erase(key);
  }

  void visit(void (*visitor)(void *, const uint8_t *, size_t), void *state) {
    const std::scoped_lock lock(mutex);
    for (const auto &[key, sections] : copied) {
      for (const std::string &section : sections) {
        const StringRef bytes(section);
        visitor(state, bytes.bytes_begin(), bytes.size());
      }
    }
    for (const auto &[key, sections] : loaded) {
      for (const auto &[address, size] : sections) {
        // The address RuntimeDyld gave is this process's, as a number.
        visitor(state, jitTargetAddressToPointer<const uint8_t *>(address),
                size);
      }
    }
  }

private:
  std::mutex mutex;
  std::map<orc::ResourceKey, std::vector<std::string>> copied;
  std::map<ObjectKey, std::vector<std::pair<uint64_t, uint64_t>>> loaded;
};

// Keeps a stack map section through JITLink's dead stripping, and copies it
// once it is relocated.
class StackMapPlugin final : public orc::LinkGraphLinkingLayer::Plugin {
public:
  explicit StackMapPlugin(CapturedStackMaps &captured) : captured(captured) {}

  void modifyPassConfig(orc::MaterializationResponsibility &responsibility,
                        jitlink::LinkGraph & /*graph*/,
                        jitlink::PassConfiguration &config) override {
    config.PrePrunePasses.emplace_back([](jitlink::LinkGraph &graph) {
      for (const StringRef name :
           {StringRef(MachOStackMaps), StringRef(ElfStackMaps)}) {
        jitlink::Section *section = graph.findSectionByName(name);
        if (section == nullptr) {
          continue;
        }
        for (jitlink::Block *block : section->blocks()) {
          graph.addAnonymousSymbol(*block, 0, block->getSize(), false, true);
        }
      }
      return Error::success();
    });
    config.PostFixupPasses.emplace_back(
        [this, &responsibility](jitlink::LinkGraph &graph) {
          for (const StringRef name :
               {StringRef(MachOStackMaps), StringRef(ElfStackMaps)}) {
            const jitlink::Section *section = graph.findSectionByName(name);
            if (section == nullptr) {
              continue;
            }
            std::string bytes;
            for (const jitlink::Block *block : section->blocks()) {
              const ArrayRef<char> content = block->getContent();
              bytes.append(content.begin(), content.end());
            }
            const std::scoped_lock lock(mutex);
            pending[&responsibility].push_back(std::move(bytes));
          }
          return Error::success();
        });
  }

  Error
  notifyEmitted(orc::MaterializationResponsibility &responsibility) override {
    std::vector<std::string> sections;
    {
      const std::scoped_lock lock(mutex);
      const auto found = pending.find(&responsibility);
      if (found == pending.end()) {
        return Error::success();
      }
      sections = std::move(found->second);
      pending.erase(found);
    }
    return responsibility.withResourceKeyDo(
        [this, &sections](orc::ResourceKey key) {
          captured.addCopied(key, std::move(sections));
        });
  }

  Error
  notifyFailed(orc::MaterializationResponsibility &responsibility) override {
    const std::scoped_lock lock(mutex);
    pending.erase(&responsibility);
    return Error::success();
  }

  Error notifyRemovingResources(orc::JITDylib & /*library*/,
                                orc::ResourceKey key) override {
    captured.removeCopied(key);
    return Error::success();
  }

  void notifyTransferringResources(orc::JITDylib & /*library*/,
                                   orc::ResourceKey destination,
                                   orc::ResourceKey source) override {
    captured.transferCopied(destination, source);
  }

private:
  CapturedStackMaps &captured;
  std::mutex mutex;
  std::map<orc::MaterializationResponsibility *, std::vector<std::string>>
      pending;
};

} // namespace

// LLVM's C API has no other way to the JIT behind its handle. Rule 9's
// exception: this is LLVM's own conversion macro.
// NOLINTNEXTLINE(cppcoreguidelines-pro-type-reinterpret-cast)
DEFINE_SIMPLE_CONVERSION_FUNCTIONS(orc::LLJIT, LLVMOrcLLJITRef)

// Contract: `jit` is live, and outlives what this gives back, which the caller
// owns and frees with `lyftar_stack_maps_dispose` after disposing of the JIT.
extern "C" void *lyftar_jit_capture_stack_maps(LLVMOrcLLJITRef jit) {
  require(jit != nullptr, "lyftar_jit_capture_stack_maps: null");
  auto captured = std::make_unique<CapturedStackMaps>();
  orc::ObjectLayer &layer = unwrap(jit)->getObjLinkingLayer();
  if (auto *linking = dyn_cast<orc::ObjectLinkingLayer>(&layer)) {
    linking->addPlugin(std::make_shared<StackMapPlugin>(*captured));
  } else if (auto *dyld = dyn_cast<orc::RTDyldObjectLinkingLayer>(&layer)) {
    dyld->registerJITEventListener(*captured);
  }
  return captured.release();
}

// Contract: `engine` is live, and outlives what this gives back, which the
// caller owns and frees with `lyftar_stack_maps_dispose` after disposing of the
// engine.
extern "C" void *
lyftar_execution_engine_capture_stack_maps(LLVMExecutionEngineRef engine) {
  require(engine != nullptr,
          "lyftar_execution_engine_capture_stack_maps: null");
  auto captured = std::make_unique<CapturedStackMaps>();
  unwrap(engine)->RegisterJITEventListener(captured.get());
  return captured.release();
}

// Contract: `engine` is live. Makes and relocates all the code it has been
// given.
extern "C" void
lyftar_execution_engine_finish_code(LLVMExecutionEngineRef engine) {
  require(engine != nullptr, "lyftar_execution_engine_finish_code: null");
  unwrap(engine)->finalizeObject();
}

// Contract: `captured` came from one of the capture functions, and its JIT or
// engine is still live. Calls `visitor` with `state` and each section's bytes,
// which last for the call. `visitor` must not call back into this capture.
extern "C" void lyftar_stack_maps_visit(void *captured,
                                        void (*visitor)(void *, const uint8_t *,
                                                        size_t),
                                        void *state) {
  require(captured != nullptr && visitor != nullptr,
          "lyftar_stack_maps_visit: null");
  static_cast<CapturedStackMaps *>(captured)->visit(visitor, state);
}

// Contract: `captured` came from one of the capture functions, its JIT or
// engine is gone, and it isn't used afterwards.
extern "C" void lyftar_stack_maps_dispose(void *captured) {
  require(captured != nullptr, "lyftar_stack_maps_dispose: null");
  const std::unique_ptr<CapturedStackMaps> owned(
      static_cast<CapturedStackMaps *>(captured));
}
