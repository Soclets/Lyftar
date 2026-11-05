//! The plain enumerations of LLVM's C API, spelled out (D5).
//!
//! Each one can hold a value this Lyftar doesn't name, which only a mismatched LLVM could produce: LLVM's own value is
//! kept in `Unknown` rather than turning it into undefined behaviour.

use crate::raw;

/// Declares an enumeration that converts to and from a bindgen newtype enum.
macro_rules! llvm_enum {
    ($(#[$meta:meta])* pub enum $name:ident: $raw:ident { $($(#[$variant_meta:meta])* $variant:ident = $raw_variant:ident,)* }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$variant_meta])* $variant,)*
            /// A value this Lyftar doesn't name.
            Unknown(u32),
        }

        impl $name {
            #[allow(dead_code)]
            pub(crate) fn to_raw(self) -> raw::$raw {
                match self {
                    $(Self::$variant => raw::$raw::$raw_variant,)*
                    Self::Unknown(value) => raw::$raw(value),
                }
            }

            #[allow(dead_code)]
            pub(crate) fn from_raw(value: raw::$raw) -> Self {
                match value {
                    $(raw::$raw::$raw_variant => Self::$variant,)*
                    raw::$raw(other) => Self::Unknown(other),
                }
            }
        }
    };
}
pub(crate) use llvm_enum;

llvm_enum! {
    /// An instruction's operation.
    pub enum Opcode: LLVMOpcode {
        /// `ret`.
        Return = LLVMRet,
        /// `br` with no condition.
        UnconditionalBranch = LLVMUncondBr,
        /// `br` with a condition.
        ConditionalBranch = LLVMCondBr,
        /// `switch`.
        Switch = LLVMSwitch,
        /// `indirectbr`.
        IndirectBranch = LLVMIndirectBr,
        /// `invoke`.
        Invoke = LLVMInvoke,
        /// `unreachable`.
        Unreachable = LLVMUnreachable,
        /// `callbr`.
        CallBranch = LLVMCallBr,
        /// `fneg`.
        FloatNegate = LLVMFNeg,
        /// `add`.
        IntegerAdd = LLVMAdd,
        /// `fadd`.
        FloatAdd = LLVMFAdd,
        /// `sub`.
        IntegerSubtract = LLVMSub,
        /// `fsub`.
        FloatSubtract = LLVMFSub,
        /// `mul`.
        IntegerMultiply = LLVMMul,
        /// `fmul`.
        FloatMultiply = LLVMFMul,
        /// `udiv`.
        UnsignedDivide = LLVMUDiv,
        /// `sdiv`.
        SignedDivide = LLVMSDiv,
        /// `fdiv`.
        FloatDivide = LLVMFDiv,
        /// `urem`.
        UnsignedRemainder = LLVMURem,
        /// `srem`.
        SignedRemainder = LLVMSRem,
        /// `frem`.
        FloatRemainder = LLVMFRem,
        /// `shl`.
        ShiftLeft = LLVMShl,
        /// `lshr`.
        LogicalShiftRight = LLVMLShr,
        /// `ashr`.
        ArithmeticShiftRight = LLVMAShr,
        /// `and`.
        And = LLVMAnd,
        /// `or`.
        Or = LLVMOr,
        /// `xor`.
        ExclusiveOr = LLVMXor,
        /// `alloca`.
        StackAllocate = LLVMAlloca,
        /// `load`.
        Load = LLVMLoad,
        /// `store`.
        Store = LLVMStore,
        /// `getelementptr`.
        GetElementPointer = LLVMGetElementPtr,
        /// `trunc`.
        Truncate = LLVMTrunc,
        /// `zext`.
        ZeroExtend = LLVMZExt,
        /// `sext`.
        SignExtend = LLVMSExt,
        /// `fptoui`.
        FloatToUnsignedInteger = LLVMFPToUI,
        /// `fptosi`.
        FloatToSignedInteger = LLVMFPToSI,
        /// `uitofp`.
        UnsignedIntegerToFloat = LLVMUIToFP,
        /// `sitofp`.
        SignedIntegerToFloat = LLVMSIToFP,
        /// `fptrunc`.
        FloatTruncate = LLVMFPTrunc,
        /// `fpext`.
        FloatExtend = LLVMFPExt,
        /// `ptrtoint`.
        PointerToInteger = LLVMPtrToInt,
        /// `ptrtoaddr`.
        PointerToAddress = LLVMPtrToAddr,
        /// `inttoptr`.
        IntegerToPointer = LLVMIntToPtr,
        /// `bitcast`.
        BitCast = LLVMBitCast,
        /// `addrspacecast`.
        AddressSpaceCast = LLVMAddrSpaceCast,
        /// `icmp`.
        IntegerCompare = LLVMICmp,
        /// `fcmp`.
        FloatCompare = LLVMFCmp,
        /// `phi`.
        Phi = LLVMPHI,
        /// `call`.
        Call = LLVMCall,
        /// `select`.
        Select = LLVMSelect,
        /// LLVM's first reserved user operation.
        UserOperation1 = LLVMUserOp1,
        /// LLVM's second reserved user operation.
        UserOperation2 = LLVMUserOp2,
        /// `va_arg`.
        VariableArgument = LLVMVAArg,
        /// `extractelement`.
        ExtractElement = LLVMExtractElement,
        /// `insertelement`.
        InsertElement = LLVMInsertElement,
        /// `shufflevector`.
        ShuffleVector = LLVMShuffleVector,
        /// `extractvalue`.
        ExtractValue = LLVMExtractValue,
        /// `insertvalue`.
        InsertValue = LLVMInsertValue,
        /// `freeze`.
        Freeze = LLVMFreeze,
        /// `fence`.
        Fence = LLVMFence,
        /// `cmpxchg`.
        AtomicCompareExchange = LLVMAtomicCmpXchg,
        /// `atomicrmw`.
        AtomicReadModifyWrite = LLVMAtomicRMW,
        /// `resume`.
        Resume = LLVMResume,
        /// `landingpad`.
        LandingPad = LLVMLandingPad,
        /// `cleanupret`.
        CleanupReturn = LLVMCleanupRet,
        /// `catchret`.
        CatchReturn = LLVMCatchRet,
        /// `catchpad`.
        CatchPad = LLVMCatchPad,
        /// `cleanuppad`.
        CleanupPad = LLVMCleanupPad,
        /// `catchswitch`.
        CatchSwitch = LLVMCatchSwitch,
    }
}

llvm_enum! {
    /// What kind of LLVM type a type is.
    pub enum TypeKind: LLVMTypeKind {
        /// `void`.
        Void = LLVMVoidTypeKind,
        /// `half`.
        Ieee754Binary16 = LLVMHalfTypeKind,
        /// `float`.
        Ieee754Binary32 = LLVMFloatTypeKind,
        /// `double`.
        Ieee754Binary64 = LLVMDoubleTypeKind,
        /// `x86_fp80`.
        X87Extended = LLVMX86_FP80TypeKind,
        /// `fp128`.
        Ieee754Binary128 = LLVMFP128TypeKind,
        /// `ppc_fp128`.
        PowerPcDoubleDouble = LLVMPPC_FP128TypeKind,
        /// `label`.
        Label = LLVMLabelTypeKind,
        /// An integer.
        Integer = LLVMIntegerTypeKind,
        /// A function type.
        Function = LLVMFunctionTypeKind,
        /// A struct.
        Struct = LLVMStructTypeKind,
        /// An array.
        Array = LLVMArrayTypeKind,
        /// A pointer.
        Pointer = LLVMPointerTypeKind,
        /// A fixed vector.
        Vector = LLVMVectorTypeKind,
        /// `metadata`.
        Metadata = LLVMMetadataTypeKind,
        /// `token`.
        Token = LLVMTokenTypeKind,
        /// A scalable vector.
        ScalableVector = LLVMScalableVectorTypeKind,
        /// `bfloat`.
        BrainFloat = LLVMBFloatTypeKind,
        /// `x86_amx`.
        X86AdvancedMatrixExtensions = LLVMX86_AMXTypeKind,
        /// A target extension type.
        TargetExtension = LLVMTargetExtTypeKind,
        /// LLVM 23's byte type.
        Byte = LLVMByteTypeKind,
    }
}

llvm_enum! {
    /// A global's linkage.
    pub enum Linkage: LLVMLinkage {
        /// `external`.
        External = LLVMExternalLinkage,
        /// `available_externally`.
        AvailableExternally = LLVMAvailableExternallyLinkage,
        /// `linkonce`.
        LinkOnceAny = LLVMLinkOnceAnyLinkage,
        /// `linkonce_odr`.
        LinkOnceOneDefinitionRule = LLVMLinkOnceODRLinkage,
        /// `weak`.
        WeakAny = LLVMWeakAnyLinkage,
        /// `weak_odr`.
        WeakOneDefinitionRule = LLVMWeakODRLinkage,
        /// `appending`.
        Appending = LLVMAppendingLinkage,
        /// `internal`.
        Internal = LLVMInternalLinkage,
        /// `private`.
        Private = LLVMPrivateLinkage,
        /// `extern_weak`.
        ExternalWeak = LLVMExternalWeakLinkage,
        /// `common`.
        Common = LLVMCommonLinkage,
    }
}

llvm_enum! {
    /// A global's visibility.
    pub enum Visibility: LLVMVisibility {
        /// `default`.
        Default = LLVMDefaultVisibility,
        /// `hidden`.
        Hidden = LLVMHiddenVisibility,
        /// `protected`.
        Protected = LLVMProtectedVisibility,
    }
}

llvm_enum! {
    /// Whether a global's address is significant.
    pub enum UnnamedAddress: LLVMUnnamedAddr {
        /// The address is significant.
        None = LLVMNoUnnamedAddr,
        /// The address isn't significant in this module.
        Local = LLVMLocalUnnamedAddr,
        /// The address isn't significant anywhere.
        Global = LLVMGlobalUnnamedAddr,
    }
}

llvm_enum! {
    /// A global's Windows DLL storage class.
    pub enum DllStorageClass: LLVMDLLStorageClass {
        /// Neither imported nor exported.
        Default = LLVMDefaultStorageClass,
        /// `dllimport`.
        Import = LLVMDLLImportStorageClass,
        /// `dllexport`.
        Export = LLVMDLLExportStorageClass,
    }
}

llvm_enum! {
    /// A thread-local global's model.
    pub enum ThreadLocalMode: LLVMThreadLocalMode {
        /// Not thread-local.
        NotThreadLocal = LLVMNotThreadLocal,
        /// General dynamic.
        GeneralDynamic = LLVMGeneralDynamicTLSModel,
        /// Local dynamic.
        LocalDynamic = LLVMLocalDynamicTLSModel,
        /// Initial exec.
        InitialExec = LLVMInitialExecTLSModel,
        /// Local exec.
        LocalExec = LLVMLocalExecTLSModel,
    }
}

llvm_enum! {
    /// An integer comparison.
    pub enum IntegerPredicate: LLVMIntPredicate {
        /// `eq`.
        Equal = LLVMIntEQ,
        /// `ne`.
        NotEqual = LLVMIntNE,
        /// `ugt`.
        UnsignedGreaterThan = LLVMIntUGT,
        /// `uge`.
        UnsignedGreaterOrEqual = LLVMIntUGE,
        /// `ult`.
        UnsignedLessThan = LLVMIntULT,
        /// `ule`.
        UnsignedLessOrEqual = LLVMIntULE,
        /// `sgt`.
        SignedGreaterThan = LLVMIntSGT,
        /// `sge`.
        SignedGreaterOrEqual = LLVMIntSGE,
        /// `slt`.
        SignedLessThan = LLVMIntSLT,
        /// `sle`.
        SignedLessOrEqual = LLVMIntSLE,
    }
}

llvm_enum! {
    /// A floating-point comparison.
    pub enum FloatPredicate: LLVMRealPredicate {
        /// Always false.
        False = LLVMRealPredicateFalse,
        /// `oeq`.
        OrderedEqual = LLVMRealOEQ,
        /// `ogt`.
        OrderedGreaterThan = LLVMRealOGT,
        /// `oge`.
        OrderedGreaterOrEqual = LLVMRealOGE,
        /// `olt`.
        OrderedLessThan = LLVMRealOLT,
        /// `ole`.
        OrderedLessOrEqual = LLVMRealOLE,
        /// `one`.
        OrderedNotEqual = LLVMRealONE,
        /// `ord`.
        Ordered = LLVMRealORD,
        /// `uno`.
        Unordered = LLVMRealUNO,
        /// `ueq`.
        UnorderedEqual = LLVMRealUEQ,
        /// `ugt`.
        UnorderedGreaterThan = LLVMRealUGT,
        /// `uge`.
        UnorderedGreaterOrEqual = LLVMRealUGE,
        /// `ult`.
        UnorderedLessThan = LLVMRealULT,
        /// `ule`.
        UnorderedLessOrEqual = LLVMRealULE,
        /// `une`.
        UnorderedNotEqual = LLVMRealUNE,
        /// Always true.
        True = LLVMRealPredicateTrue,
    }
}

llvm_enum! {
    /// How an atomic operation orders memory.
    pub enum AtomicOrdering: LLVMAtomicOrdering {
        /// Not atomic.
        NotAtomic = LLVMAtomicOrderingNotAtomic,
        /// `unordered`.
        Unordered = LLVMAtomicOrderingUnordered,
        /// `monotonic`.
        Monotonic = LLVMAtomicOrderingMonotonic,
        /// `acquire`.
        Acquire = LLVMAtomicOrderingAcquire,
        /// `release`.
        Release = LLVMAtomicOrderingRelease,
        /// `acq_rel`.
        AcquireRelease = LLVMAtomicOrderingAcquireRelease,
        /// `seq_cst`.
        SequentiallyConsistent = LLVMAtomicOrderingSequentiallyConsistent,
    }
}

llvm_enum! {
    /// What an `atomicrmw` does.
    pub enum AtomicReadModifyWriteOperation: LLVMAtomicRMWBinOp {
        /// `xchg`.
        Exchange = LLVMAtomicRMWBinOpXchg,
        /// `add`.
        Add = LLVMAtomicRMWBinOpAdd,
        /// `sub`.
        Subtract = LLVMAtomicRMWBinOpSub,
        /// `and`.
        And = LLVMAtomicRMWBinOpAnd,
        /// `nand`.
        NotAnd = LLVMAtomicRMWBinOpNand,
        /// `or`.
        Or = LLVMAtomicRMWBinOpOr,
        /// `xor`.
        ExclusiveOr = LLVMAtomicRMWBinOpXor,
        /// `max`.
        SignedMaximum = LLVMAtomicRMWBinOpMax,
        /// `min`.
        SignedMinimum = LLVMAtomicRMWBinOpMin,
        /// `umax`.
        UnsignedMaximum = LLVMAtomicRMWBinOpUMax,
        /// `umin`.
        UnsignedMinimum = LLVMAtomicRMWBinOpUMin,
        /// `fadd`.
        FloatAdd = LLVMAtomicRMWBinOpFAdd,
        /// `fsub`.
        FloatSubtract = LLVMAtomicRMWBinOpFSub,
        /// `fmax`.
        FloatMaximum = LLVMAtomicRMWBinOpFMax,
        /// `fmin`.
        FloatMinimum = LLVMAtomicRMWBinOpFMin,
        /// `uinc_wrap`.
        UnsignedIncrementWrap = LLVMAtomicRMWBinOpUIncWrap,
        /// `udec_wrap`.
        UnsignedDecrementWrap = LLVMAtomicRMWBinOpUDecWrap,
        /// `usub_cond`.
        UnsignedSubtractConditional = LLVMAtomicRMWBinOpUSubCond,
        /// `usub_sat`.
        UnsignedSubtractSaturate = LLVMAtomicRMWBinOpUSubSat,
        /// `fmaximum`.
        FloatMaximumIeee754 = LLVMAtomicRMWBinOpFMaximum,
        /// `fminimum`.
        FloatMinimumIeee754 = LLVMAtomicRMWBinOpFMinimum,
        /// `fmaximumnum`.
        FloatMaximumNumber = LLVMAtomicRMWBinOpFMaximumNum,
        /// `fminimumnum`.
        FloatMinimumNumber = LLVMAtomicRMWBinOpFMinimumNum,
    }
}

llvm_enum! {
    /// A call's tail-call marker.
    pub enum TailCallKind: LLVMTailCallKind {
        /// None.
        None = LLVMTailCallKindNone,
        /// `tail`.
        Tail = LLVMTailCallKindTail,
        /// `musttail`.
        MustTail = LLVMTailCallKindMustTail,
        /// `notail`.
        NoTail = LLVMTailCallKindNoTail,
    }
}

llvm_enum! {
    /// Which syntax inline assembly is written in.
    pub enum InlineAssemblyDialect: LLVMInlineAsmDialect {
        /// AT&T.
        Att = LLVMInlineAsmDialectATT,
        /// Intel.
        Intel = LLVMInlineAsmDialectIntel,
    }
}

llvm_enum! {
    /// What the linker does when two modules set the same module flag.
    pub enum ModuleFlagBehavior: LLVMModuleFlagBehavior {
        /// Emit an error.
        Error = LLVMModuleFlagBehaviorError,
        /// Emit a warning.
        Warning = LLVMModuleFlagBehaviorWarning,
        /// Require another flag to have a value.
        Require = LLVMModuleFlagBehaviorRequire,
        /// Use the second module's value.
        Override = LLVMModuleFlagBehaviorOverride,
        /// Append the lists.
        Append = LLVMModuleFlagBehaviorAppend,
        /// Append the lists, keeping each item once.
        AppendUnique = LLVMModuleFlagBehaviorAppendUnique,
    }
}

llvm_enum! {
    /// How serious a diagnostic is.
    pub enum DiagnosticSeverity: LLVMDiagnosticSeverity {
        /// An error.
        Error = LLVMDSError,
        /// A warning.
        Warning = LLVMDSWarning,
        /// An optimization remark.
        Remark = LLVMDSRemark,
        /// A note.
        Note = LLVMDSNote,
    }
}

llvm_enum! {
    /// What kind of LLVM value a value is.
    pub enum ValueCategory: LLVMValueKind {
        /// A function argument.
        Argument = LLVMArgumentValueKind,
        /// A basic block.
        BasicBlock = LLVMBasicBlockValueKind,
        /// A use of memory SSA.
        MemoryUse = LLVMMemoryUseValueKind,
        /// A definition in memory SSA.
        MemoryDefinition = LLVMMemoryDefValueKind,
        /// A phi in memory SSA.
        MemoryPhi = LLVMMemoryPhiValueKind,
        /// A function.
        Function = LLVMFunctionValueKind,
        /// A global alias.
        GlobalAlias = LLVMGlobalAliasValueKind,
        /// A global indirect function.
        GlobalIndirectFunction = LLVMGlobalIFuncValueKind,
        /// A global variable.
        GlobalVariable = LLVMGlobalVariableValueKind,
        /// A block's address.
        BlockAddress = LLVMBlockAddressValueKind,
        /// A constant expression.
        ConstantExpression = LLVMConstantExprValueKind,
        /// A constant array.
        ConstantArray = LLVMConstantArrayValueKind,
        /// A constant struct.
        ConstantStruct = LLVMConstantStructValueKind,
        /// A constant vector.
        ConstantVector = LLVMConstantVectorValueKind,
        /// `undef`.
        UndefinedValue = LLVMUndefValueValueKind,
        /// A zero aggregate.
        ConstantAggregateZero = LLVMConstantAggregateZeroValueKind,
        /// A constant data array.
        ConstantDataArray = LLVMConstantDataArrayValueKind,
        /// A constant data vector.
        ConstantDataVector = LLVMConstantDataVectorValueKind,
        /// A constant integer.
        ConstantInteger = LLVMConstantIntValueKind,
        /// A constant float.
        ConstantFloat = LLVMConstantFPValueKind,
        /// A constant null pointer.
        ConstantPointerNull = LLVMConstantPointerNullValueKind,
        /// A constant `none` token.
        ConstantTokenNone = LLVMConstantTokenNoneValueKind,
        /// Metadata wrapped as a value.
        MetadataAsValue = LLVMMetadataAsValueValueKind,
        /// Inline assembly.
        InlineAssembly = LLVMInlineAsmValueKind,
        /// An instruction.
        Instruction = LLVMInstructionValueKind,
        /// `poison`.
        PoisonValue = LLVMPoisonValueValueKind,
        /// A constant target-extension value that is none.
        ConstantTargetNone = LLVMConstantTargetNoneValueKind,
        /// A constant authenticated pointer.
        ConstantPointerAuthentication = LLVMConstantPtrAuthValueKind,
        /// A constant byte.
        ConstantByte = LLVMConstantByteValueKind,
    }
}
