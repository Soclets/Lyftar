// Passes in Rust: LLVM's new pass manager wants C++ classes and callbacks, so these hold a Rust pass or closure behind
// plain function pointers, and everything crossing here comes from Rust's `passes` module (D16). Each function's contract
// says who owns each pointer; unless it says otherwise, every pointer is live, non-null and borrowed for the call. A Rust
// callback catches its own panics and aborts, so none crosses these C++ frames (D3).

#include "shim.h"

#include "llvm-c/Core.h"
#include "llvm-c/Error.h"
#include "llvm-c/TargetMachine.h"
#include "llvm-c/Types.h"
#include "llvm/ADT/SmallVector.h"
#include "llvm/Analysis/AliasAnalysis.h"
#include "llvm/Analysis/CGSCCPassManager.h"
#include "llvm/Analysis/InlineCost.h"
#include "llvm/Analysis/LoopAnalysisManager.h"
#include "llvm/Analysis/LoopInfo.h"
#include "llvm/IR/Dominators.h"
#include "llvm/IR/Module.h"
#include "llvm/IR/PassManager.h"
#include "llvm/IR/StructuralHash.h"
#include "llvm/IR/ValueMap.h"
#include "llvm/IR/Verifier.h"
#include "llvm/Passes/PassBuilder.h"
#include "llvm/Passes/StandardInstrumentations.h"
#include "llvm/Support/CBindingWrapping.h"
#include "llvm/Support/Error.h"
#include "llvm/Target/TargetMachine.h"
#include "llvm/Transforms/Scalar/LoopPassManager.h"
#include "llvm/Transforms/Utils/Cloning.h"
#include "llvm/Transforms/Utils/ValueMapper.h"

#include <cstddef>
#include <cstdint>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <utility>

using namespace llvm;
using lyftar::require;

namespace {

using RunModule = bool (*)(void *, LLVMModuleRef, void *);
using RunFunction = bool (*)(void *, LLVMValueRef, void *);
using Dispose = void (*)(void *);
using MakePass = void (*)(void *, void *, void *);
using RegisterCall = void (*)(void *, void *);
using ExtensionPointCall = void (*)(void *, void *, void *, uint32_t, uint32_t);

// Rust state, freed by Rust. It moves with the pass that holds it.
class OwnedState {
public:
  OwnedState(void *state, Dispose dispose) : State(state), Free(dispose) {}
  OwnedState(const OwnedState &) = delete;
  OwnedState &operator=(const OwnedState &) = delete;
  OwnedState(OwnedState &&other) noexcept
      : State(std::exchange(other.State, nullptr)), Free(other.Free) {}
  OwnedState &operator=(OwnedState &&other) noexcept {
    std::swap(State, other.State);
    std::swap(Free, other.Free);
    return *this;
  }
  ~OwnedState() {
    if (State != nullptr) {
      Free(State);
    }
  }
  void *get() const { return State; }

private:
  void *State;
  Dispose Free;
};

// A module pass whose body is Rust. The function analyses go to Rust, since a module pass asks for them by function.
class RustModulePass : public PassInfoMixin<RustModulePass> {
public:
  RustModulePass(void *state, RunModule run, Dispose dispose)
      : State(state, dispose), Run(run) {}

  PreservedAnalyses run(Module &module, ModuleAnalysisManager &analyses) {
    FunctionAnalysisManager &functions =
        analyses.getResult<FunctionAnalysisManagerModuleProxy>(module)
            .getManager();
    return Run(State.get(), wrap(&module), &functions)
               ? PreservedAnalyses::all()
               : PreservedAnalyses::none();
  }

private:
  OwnedState State;
  RunModule Run;
};

// A function pass whose body is Rust.
class RustFunctionPass : public PassInfoMixin<RustFunctionPass> {
public:
  RustFunctionPass(void *state, RunFunction run, Dispose dispose)
      : State(state, dispose), Run(run) {}

  PreservedAnalyses run(Function &function,
                        FunctionAnalysisManager &analyses) {
    return Run(State.get(), wrap(&function), &analyses)
               ? PreservedAnalyses::all()
               : PreservedAnalyses::none();
  }

private:
  OwnedState State;
  RunFunction Run;
};

uint32_t levelNumber(OptimizationLevel level) {
  return static_cast<uint32_t>(level);
}

} // namespace

// What Rust's `PassOptions` says.
struct LyftarPassOptions {
  bool VerifyEach;
  bool DebugLogging;
  bool LoopInterleaving;
  bool LoopVectorization;
  bool SuperwordLevelParallelismVectorization;
  bool LoopUnrolling;
  bool ForgetAllScalarEvolutionInLoopUnroll;
  bool CallGraphProfile;
  bool MergeFunctions;
  uint32_t LicmMemorySsaOptimizationCap;
  uint32_t LicmMemorySsaNoAccessForPromotionCap;
  int32_t InlinerThreshold;
  const char *AliasAnalysisPipeline;
  size_t AliasAnalysisPipelineLength;
};

// Contract: `module` is live, and the caller holds exclusive access to it. `pipeline` has `length` bytes of LLVM's pipeline
// text. `machine` can be null. `state` goes to `call`, which registers Rust's passes and callbacks on the pass builder
// before the text is parsed. Those live until this call returns. Returns null on success, and otherwise an error the
// caller owns.
extern "C" LLVMErrorRef
lyftar_run_passes_with(LLVMModuleRef module, const char *pipeline, size_t length,
                       LLVMTargetMachineRef machine,
                       const LyftarPassOptions *options, void *state,
                       RegisterCall call) {
  require(module != nullptr && (pipeline != nullptr || length == 0) &&
              options != nullptr && call != nullptr,
          "lyftar_run_passes_with: bad argument");
  PipelineTuningOptions tuning;
  tuning.LoopInterleaving = options->LoopInterleaving;
  tuning.LoopVectorization = options->LoopVectorization;
  tuning.SLPVectorization = options->SuperwordLevelParallelismVectorization;
  tuning.LoopUnrolling = options->LoopUnrolling;
  tuning.ForgetAllSCEVInLoopUnroll =
      options->ForgetAllScalarEvolutionInLoopUnroll;
  tuning.LicmMssaOptCap = options->LicmMemorySsaOptimizationCap;
  tuning.LicmMssaNoAccForPromotionCap =
      options->LicmMemorySsaNoAccessForPromotionCap;
  tuning.CallGraphProfile = options->CallGraphProfile;
  tuning.MergeFunctions = options->MergeFunctions;
  tuning.InlinerThreshold = options->InlinerThreshold;

  // LLVM's C API gives no other way to get from its handle to the target machine.
  auto *target_machine = reinterpret_cast<TargetMachine *>(machine);
  PassInstrumentationCallbacks instrumentation;
  PassBuilder builder(target_machine, tuning, std::nullopt, &instrumentation);

  LoopAnalysisManager loops;
  FunctionAnalysisManager functions;
  CGSCCAnalysisManager call_graph;
  ModuleAnalysisManager modules;
  if (options->AliasAnalysisPipelineLength != 0) {
    AAManager alias_analysis;
    if (Error failure = builder.parseAAPipeline(
            alias_analysis, StringRef(options->AliasAnalysisPipeline,
                                      options->AliasAnalysisPipelineLength))) {
      return wrap(std::move(failure));
    }
    functions.registerPass([&alias_analysis] { return std::move(alias_analysis); });
  }
  builder.registerLoopAnalyses(loops);
  builder.registerFunctionAnalyses(functions);
  builder.registerCGSCCAnalyses(call_graph);
  builder.registerModuleAnalyses(modules);
  builder.crossRegisterProxies(loops, functions, call_graph, modules);

  Module *found = unwrap(module);
  StandardInstrumentations standard(found->getContext(), options->DebugLogging,
                                    options->VerifyEach);
  standard.registerCallbacks(instrumentation, &modules);

  call(state, &builder);

  ModulePassManager passes;
  if (options->VerifyEach) {
    passes.addPass(VerifierPass());
  }
  if (Error failure =
          builder.parsePassPipeline(passes, StringRef(pipeline, length))) {
    return wrap(std::move(failure));
  }
  passes.run(*found, modules);
  return nullptr;
}

// Contract: `builder` is a `PassBuilder`, `name` has `length` bytes, and this call owns `state`, freed with `dispose`. When
// pipeline text names the pass, `make` gets the state, the builder and the module pass manager to add it to. The name is
// copied.
extern "C" void lyftar_pass_builder_register_module_pass(void *builder,
                                                         const char *name,
                                                         size_t length,
                                                         void *state,
                                                         MakePass make,
                                                         Dispose dispose) {
  require(builder != nullptr && (name != nullptr || length == 0) &&
              make != nullptr && dispose != nullptr,
          "lyftar_pass_builder_register_module_pass: bad argument");
  auto *passes = static_cast<PassBuilder *>(builder);
  auto owned = std::make_shared<OwnedState>(state, dispose);
  passes->registerPipelineParsingCallback(
      [owned, make, passes, wanted = std::string(name, length)](
          StringRef asked, ModulePassManager &manager,
          ArrayRef<PassBuilder::PipelineElement>) {
        if (asked != wanted) {
          return false;
        }
        make(owned->get(), passes, &manager);
        return true;
      });
}

// Contract: as `lyftar_pass_builder_register_module_pass`, for a function pass. `make` gets the function pass manager.
extern "C" void lyftar_pass_builder_register_function_pass(void *builder,
                                                           const char *name,
                                                           size_t length,
                                                           void *state,
                                                           MakePass make,
                                                           Dispose dispose) {
  require(builder != nullptr && (name != nullptr || length == 0) &&
              make != nullptr && dispose != nullptr,
          "lyftar_pass_builder_register_function_pass: bad argument");
  auto *passes = static_cast<PassBuilder *>(builder);
  auto owned = std::make_shared<OwnedState>(state, dispose);
  passes->registerPipelineParsingCallback(
      [owned, make, passes, wanted = std::string(name, length)](
          StringRef asked, FunctionPassManager &manager,
          ArrayRef<PassBuilder::PipelineElement>) {
        if (asked != wanted) {
          return false;
        }
        make(owned->get(), passes, &manager);
        return true;
      });
}

// Contract: `builder` is a `PassBuilder`, and this call owns `state`, freed with `dispose`. `which` picks LLVM's extension
// point: 0 peephole, 1 late loop optimizations, 2 loop optimizer end, 3 scalar optimizer late, 4 call graph optimizer late,
// 5 vectorizer start, 6 vectorizer end, 7 pipeline start, 8 pipeline early simplification, 9 optimizer early, 10 optimizer
// last, 11 full link-time optimization early, 12 full link-time optimization last. `call` gets the state, the builder, the
// point's pass manager, the optimization level (0 to 3) and the link-time phase (0 none, 1
// thin pre-link, 2 thin post-link, 3 full pre-link, 4 full post-link).
extern "C" void lyftar_pass_builder_register_extension_point(
    void *builder, uint32_t which, void *state, ExtensionPointCall call,
    Dispose dispose) {
  require(builder != nullptr && call != nullptr && dispose != nullptr,
          "lyftar_pass_builder_register_extension_point: bad argument");
  auto *passes = static_cast<PassBuilder *>(builder);
  auto owned = std::make_shared<OwnedState>(state, dispose);
  const auto simple = [owned, call, passes](auto &manager,
                                            OptimizationLevel level) {
    call(owned->get(), passes, &manager, levelNumber(level), 0);
  };
  const auto phased = [owned, call, passes](ModulePassManager &manager,
                                            OptimizationLevel level,
                                            ThinOrFullLTOPhase phase) {
    call(owned->get(), passes, &manager, levelNumber(level),
         static_cast<uint32_t>(phase));
  };
  switch (which) {
  case 0:
    passes->registerPeepholeEPCallback(simple);
    break;
  case 1:
    passes->registerLateLoopOptimizationsEPCallback(simple);
    break;
  case 2:
    passes->registerLoopOptimizerEndEPCallback(simple);
    break;
  case 3:
    passes->registerScalarOptimizerLateEPCallback(simple);
    break;
  case 4:
    passes->registerCGSCCOptimizerLateEPCallback(simple);
    break;
  case 5:
    passes->registerVectorizerStartEPCallback(simple);
    break;
  case 6:
    passes->registerVectorizerEndEPCallback(simple);
    break;
  case 7:
    passes->registerPipelineStartEPCallback(simple);
    break;
  case 8:
    passes->registerPipelineEarlySimplificationEPCallback(phased);
    break;
  case 9:
    passes->registerOptimizerEarlyEPCallback(phased);
    break;
  case 10:
    passes->registerOptimizerLastEPCallback(phased);
    break;
  case 11:
    passes->registerFullLinkTimeOptimizationEarlyEPCallback(simple);
    break;
  case 12:
    passes->registerFullLinkTimeOptimizationLastEPCallback(simple);
    break;
  default:
    require(false, "lyftar_pass_builder_register_extension_point: bad point");
  }
}

// Contract: `manager` is a `ModulePassManager`, and the pass owns `state` from here on, freed with `dispose`.
extern "C" void lyftar_module_pass_manager_add_module_pass(void *manager,
                                                           void *state,
                                                           RunModule run,
                                                           Dispose dispose) {
  require(manager != nullptr && run != nullptr && dispose != nullptr,
          "lyftar_module_pass_manager_add_module_pass: bad argument");
  static_cast<ModulePassManager *>(manager)->addPass(
      RustModulePass(state, run, dispose));
}

// Contract: as above, for a function pass run on each function of the module.
extern "C" void lyftar_module_pass_manager_add_function_pass(void *manager,
                                                             void *state,
                                                             RunFunction run,
                                                             Dispose dispose) {
  require(manager != nullptr && run != nullptr && dispose != nullptr,
          "lyftar_module_pass_manager_add_function_pass: bad argument");
  static_cast<ModulePassManager *>(manager)->addPass(
      createModuleToFunctionPassAdaptor(RustFunctionPass(state, run, dispose)));
}

// Contract: `manager` is a `FunctionPassManager`, and the pass owns `state` from here on, freed with `dispose`.
extern "C" void lyftar_function_pass_manager_add_function_pass(void *manager,
                                                               void *state,
                                                               RunFunction run,
                                                               Dispose dispose) {
  require(manager != nullptr && run != nullptr && dispose != nullptr,
          "lyftar_function_pass_manager_add_function_pass: bad argument");
  static_cast<FunctionPassManager *>(manager)->addPass(
      RustFunctionPass(state, run, dispose));
}

// Contract: `manager` is a `CGSCCPassManager`, and the pass owns `state` from here on, freed with `dispose`.
extern "C" void lyftar_call_graph_pass_manager_add_function_pass(void *manager,
                                                                 void *state,
                                                                 RunFunction run,
                                                                 Dispose dispose) {
  require(manager != nullptr && run != nullptr && dispose != nullptr,
          "lyftar_call_graph_pass_manager_add_function_pass: bad argument");
  static_cast<CGSCCPassManager *>(manager)->addPass(
      createCGSCCToFunctionPassAdaptor(RustFunctionPass(state, run, dispose)));
}

// Contract: `builder` is a `PassBuilder`, `manager` is the pass manager `which` says (0 module, 1 function, 2 call graph, 3
// loop), and `pipeline` has `length` bytes of LLVM's pipeline text for that level. Returns null on success, and otherwise
// an error the caller owns.
extern "C" LLVMErrorRef lyftar_pass_manager_add_pipeline(void *builder,
                                                         void *manager,
                                                         uint32_t which,
                                                         const char *pipeline,
                                                         size_t length) {
  require(builder != nullptr && manager != nullptr &&
              (pipeline != nullptr || length == 0),
          "lyftar_pass_manager_add_pipeline: bad argument");
  auto *passes = static_cast<PassBuilder *>(builder);
  const StringRef text(pipeline, length);
  switch (which) {
  case 0:
    return wrap(
        passes->parsePassPipeline(*static_cast<ModulePassManager *>(manager), text));
  case 1:
    return wrap(passes->parsePassPipeline(
        *static_cast<FunctionPassManager *>(manager), text));
  case 2:
    return wrap(
        passes->parsePassPipeline(*static_cast<CGSCCPassManager *>(manager), text));
  case 3:
    return wrap(
        passes->parsePassPipeline(*static_cast<LoopPassManager *>(manager), text));
  default:
    require(false, "lyftar_pass_manager_add_pipeline: bad pass manager");
    return nullptr;
  }
}

// Contract: `analyses` is the `FunctionAnalysisManager` a pass was given, and no analysis result from it is in use. Drops
// them all, since the IR they describe has changed.
// trace: D11
extern "C" void lyftar_analyses_clear(void *analyses) {
  require(analyses != nullptr, "lyftar_analyses_clear: null analyses");
  static_cast<FunctionAnalysisManager *>(analyses)->clear();
}

// Contract: `analyses` is the `FunctionAnalysisManager` a pass was given, and `function` is a function with a body. Gives
// back the dominator tree, which the analysis manager owns until it is told the function changed.
extern "C" void *lyftar_analyses_dominator_tree(void *analyses,
                                                LLVMValueRef function) {
  require(analyses != nullptr && function != nullptr &&
              isa<Function>(unwrap(function)) &&
              !cast<Function>(unwrap(function))->isDeclaration(),
          "lyftar_analyses_dominator_tree: bad argument");
  return &static_cast<FunctionAnalysisManager *>(analyses)
              ->getResult<DominatorTreeAnalysis>(*cast<Function>(unwrap(function)));
}

// Contract: `tree` is a dominator tree. Gives back its root block.
extern "C" LLVMBasicBlockRef lyftar_dominator_tree_root(void *tree) {
  require(tree != nullptr, "lyftar_dominator_tree_root: null tree");
  return wrap(static_cast<DominatorTree *>(tree)->getRoot());
}

// Contract: `tree` is a dominator tree, and both blocks are in its function. Blocks added since it was made count as
// unreachable. Returns whether the first dominates the second.
extern "C" bool lyftar_dominator_tree_block_dominates(void *tree,
                                                      LLVMBasicBlockRef dominating,
                                                      LLVMBasicBlockRef dominated) {
  require(tree != nullptr && dominating != nullptr && dominated != nullptr,
          "lyftar_dominator_tree_block_dominates: null argument");
  return static_cast<DominatorTree *>(tree)->dominates(unwrap(dominating),
                                                       unwrap(dominated));
}

// Contract: as above, but a block doesn't properly dominate itself.
extern "C" bool
lyftar_dominator_tree_block_properly_dominates(void *tree,
                                               LLVMBasicBlockRef dominating,
                                               LLVMBasicBlockRef dominated) {
  require(tree != nullptr && dominating != nullptr && dominated != nullptr,
          "lyftar_dominator_tree_block_properly_dominates: null argument");
  return static_cast<DominatorTree *>(tree)->properlyDominates(unwrap(dominating),
                                                               unwrap(dominated));
}

// Contract: `tree` is a dominator tree, `definition` is any value and `user` is an instruction in the tree's function.
// Returns whether the definition dominates the user.
extern "C" bool lyftar_dominator_tree_value_dominates(void *tree,
                                                      LLVMValueRef definition,
                                                      LLVMValueRef user) {
  require(tree != nullptr && definition != nullptr && user != nullptr &&
              isa<Instruction>(unwrap(user)),
          "lyftar_dominator_tree_value_dominates: bad argument");
  return static_cast<DominatorTree *>(tree)->dominates(
      unwrap(definition), cast<Instruction>(unwrap(user)));
}

// Contract: `tree` is a dominator tree, and `block` is in its function. Gives back the immediate dominator, or null if
// there is none.
extern "C" LLVMBasicBlockRef
lyftar_dominator_tree_immediate_dominator(void *tree, LLVMBasicBlockRef block) {
  require(tree != nullptr && block != nullptr,
          "lyftar_dominator_tree_immediate_dominator: null argument");
  const DomTreeNode *node = static_cast<DominatorTree *>(tree)->getNode(unwrap(block));
  if (node == nullptr || node->getIDom() == nullptr) {
    return nullptr;
  }
  return wrap(node->getIDom()->getBlock());
}

// Contract: `tree` is a dominator tree, and `block` is in its function. Returns whether the tree knows it is reachable.
extern "C" bool lyftar_dominator_tree_is_reachable(void *tree,
                                                   LLVMBasicBlockRef block) {
  require(tree != nullptr && block != nullptr,
          "lyftar_dominator_tree_is_reachable: null argument");
  return static_cast<DominatorTree *>(tree)->isReachableFromEntry(unwrap(block));
}

// Contract: `tree` is a dominator tree, and `block` is in its function. Gives back how many blocks it immediately dominates.
extern "C" uint32_t lyftar_dominator_tree_child_count(void *tree,
                                                      LLVMBasicBlockRef block) {
  require(tree != nullptr && block != nullptr,
          "lyftar_dominator_tree_child_count: null argument");
  const DomTreeNode *node = static_cast<DominatorTree *>(tree)->getNode(unwrap(block));
  if (node == nullptr) {
    return 0;
  }
  uint32_t count = 0;
  for (auto child = node->begin(); child != node->end(); ++child) {
    ++count;
  }
  return count;
}

// Contract: as above, and `index` is below the count. Gives back that immediately dominated block.
extern "C" LLVMBasicBlockRef lyftar_dominator_tree_child_at(void *tree,
                                                            LLVMBasicBlockRef block,
                                                            uint32_t index) {
  require(tree != nullptr && block != nullptr,
          "lyftar_dominator_tree_child_at: null argument");
  const DomTreeNode *node = static_cast<DominatorTree *>(tree)->getNode(unwrap(block));
  require(node != nullptr, "lyftar_dominator_tree_child_at: no such block");
  uint32_t at = 0;
  for (const DomTreeNode *child : *node) {
    if (at == index) {
      return wrap(child->getBlock());
    }
    ++at;
  }
  require(false, "lyftar_dominator_tree_child_at: index out of bounds");
  return nullptr;
}

// Contract: `analyses` is the `FunctionAnalysisManager` a pass was given, and `function` is a function with a body. Gives
// back the loop information, which the analysis manager owns until it is told the function changed.
extern "C" void *lyftar_analyses_loop_info(void *analyses, LLVMValueRef function) {
  require(analyses != nullptr && function != nullptr &&
              isa<Function>(unwrap(function)) &&
              !cast<Function>(unwrap(function))->isDeclaration(),
          "lyftar_analyses_loop_info: bad argument");
  return &static_cast<FunctionAnalysisManager *>(analyses)->getResult<LoopAnalysis>(
      *cast<Function>(unwrap(function)));
}

// Contract: `information` is loop information, and `block` is in its function. Gives back the innermost loop holding it,
// or null. The loop information owns the loop.
extern "C" void *lyftar_loop_info_loop_for(void *information,
                                           LLVMBasicBlockRef block) {
  require(information != nullptr && block != nullptr,
          "lyftar_loop_info_loop_for: null argument");
  return static_cast<LoopInfo *>(information)->getLoopFor(unwrap(block));
}

// Contract: `information` is loop information. Gives back how many outermost loops it has.
extern "C" uint32_t lyftar_loop_info_top_level_count(void *information) {
  require(information != nullptr, "lyftar_loop_info_top_level_count: null");
  return static_cast<uint32_t>(
      static_cast<LoopInfo *>(information)->getTopLevelLoops().size());
}

// Contract: as above, and `index` is below the count. Gives back that outermost loop.
extern "C" void *lyftar_loop_info_top_level_at(void *information, uint32_t index) {
  require(information != nullptr, "lyftar_loop_info_top_level_at: null");
  const auto &loops = static_cast<LoopInfo *>(information)->getTopLevelLoops();
  require(index < loops.size(), "lyftar_loop_info_top_level_at: out of bounds");
  return loops[index];
}

// Contract: `found` is a loop. Gives back its header block.
extern "C" LLVMBasicBlockRef lyftar_loop_header(void *found) {
  require(found != nullptr, "lyftar_loop_header: null loop");
  return wrap(static_cast<Loop *>(found)->getHeader());
}

// Contract: `found` is a loop. Gives back its preheader block, or null if it has none.
extern "C" LLVMBasicBlockRef lyftar_loop_preheader(void *found) {
  require(found != nullptr, "lyftar_loop_preheader: null loop");
  return wrap(static_cast<Loop *>(found)->getLoopPreheader());
}

// Contract: `found` is a loop. Gives back its latch block, or null if it doesn't have just one.
extern "C" LLVMBasicBlockRef lyftar_loop_latch(void *found) {
  require(found != nullptr, "lyftar_loop_latch: null loop");
  return wrap(static_cast<Loop *>(found)->getLoopLatch());
}

// Contract: `found` is a loop. Gives back the loop around it, or null.
extern "C" void *lyftar_loop_parent(void *found) {
  require(found != nullptr, "lyftar_loop_parent: null loop");
  return static_cast<Loop *>(found)->getParentLoop();
}

// Contract: `found` is a loop. Gives back how deep it is, 1 for an outermost loop.
extern "C" uint32_t lyftar_loop_depth(void *found) {
  require(found != nullptr, "lyftar_loop_depth: null loop");
  return static_cast<Loop *>(found)->getLoopDepth();
}

// Contract: `found` is a loop. Returns whether `block` is in it.
extern "C" bool lyftar_loop_contains_block(void *found, LLVMBasicBlockRef block) {
  require(found != nullptr && block != nullptr,
          "lyftar_loop_contains_block: null argument");
  return static_cast<Loop *>(found)->contains(unwrap(block));
}

// Contract: `found` is a loop. Gives back how many blocks it has.
extern "C" uint32_t lyftar_loop_block_count(void *found) {
  require(found != nullptr, "lyftar_loop_block_count: null loop");
  return static_cast<Loop *>(found)->getNumBlocks();
}

// Contract: as above, and `index` is below the count. Gives back that block.
extern "C" LLVMBasicBlockRef lyftar_loop_block_at(void *found, uint32_t index) {
  require(found != nullptr, "lyftar_loop_block_at: null loop");
  const auto blocks = static_cast<Loop *>(found)->getBlocks();
  require(index < blocks.size(), "lyftar_loop_block_at: out of bounds");
  return wrap(blocks[index]);
}

// Contract: `found` is a loop. Gives back how many loops are directly inside it.
extern "C" uint32_t lyftar_loop_subloop_count(void *found) {
  require(found != nullptr, "lyftar_loop_subloop_count: null loop");
  return static_cast<uint32_t>(static_cast<Loop *>(found)->getSubLoops().size());
}

// Contract: as above, and `index` is below the count. Gives back that loop.
extern "C" void *lyftar_loop_subloop_at(void *found, uint32_t index) {
  require(found != nullptr, "lyftar_loop_subloop_at: null loop");
  const auto &loops = static_cast<Loop *>(found)->getSubLoops();
  require(index < loops.size(), "lyftar_loop_subloop_at: out of bounds");
  return loops[index];
}

// Contract: `found` is a loop, and `buffer` has room for `capacity` blocks, which can be zero with a null buffer. Writes
// as many of its exit blocks as fit, and gives back how many there are.
extern "C" uint32_t lyftar_loop_exit_blocks(void *found, LLVMBasicBlockRef *buffer,
                                            uint32_t capacity) {
  require(found != nullptr && (buffer != nullptr || capacity == 0),
          "lyftar_loop_exit_blocks: bad argument");
  SmallVector<BasicBlock *, 8> exits;
  static_cast<Loop *>(found)->getExitBlocks(exits);
  MutableArrayRef<LLVMBasicBlockRef> room(buffer, capacity);
  for (size_t index = 0; index < exits.size() && index < room.size(); ++index) {
    room[index] = wrap(exits[index]);
  }
  return static_cast<uint32_t>(exits.size());
}

// Contract: `function` is a function in a module, with a body. Adds a copy to the module and gives it back.
extern "C" LLVMValueRef lyftar_clone_function(LLVMValueRef function) {
  require(function != nullptr && isa<Function>(unwrap(function)) &&
              !cast<Function>(unwrap(function))->isDeclaration(),
          "lyftar_clone_function: bad argument");
  ValueToValueMapTy map;
  return wrap(CloneFunction(cast<Function>(unwrap(function)), map));
}

// Contract: `call` is a call of some kind in a function, and the caller holds exclusive access to its module. `message`
// points to room for one pointer, which on failure gets a message to free with `LLVMDisposeMessage`. Inlines the callee
// there and removes the call, or returns false and leaves the call as it was.
extern "C" bool lyftar_inline_call(LLVMValueRef call, char **message) {
  require(call != nullptr && isa<CallBase>(unwrap(call)) && message != nullptr,
          "lyftar_inline_call: bad argument");
  InlineFunctionInfo information;
  const InlineResult result =
      InlineFunction(*cast<CallBase>(unwrap(call)), information);
  if (result.isSuccess()) {
    return true;
  }
  *message = LLVMCreateMessage(result.getFailureReason());
  return false;
}

// Contract: gives back a hash of the module that changes when it does. Lyftar compares two to catch a pass that says it
// changed nothing, and didn't tell the truth.
extern "C" uint64_t lyftar_structural_hash_of_module(LLVMModuleRef module) {
  require(module != nullptr, "lyftar_structural_hash_of_module: null module");
  // LLVM's hash leaves out declarations, whose number still says the module changed.
  const Module &found = *unwrap(module);
  return StructuralHash(found, true) + found.size() + found.global_size() +
         found.alias_size() + found.ifunc_size();
}

// Contract: `function` is a function. As above, for it alone.
extern "C" uint64_t lyftar_structural_hash_of_function(LLVMValueRef function) {
  require(function != nullptr && isa<Function>(unwrap(function)),
          "lyftar_structural_hash_of_function: bad argument");
  return StructuralHash(*cast<Function>(unwrap(function)), true);
}
