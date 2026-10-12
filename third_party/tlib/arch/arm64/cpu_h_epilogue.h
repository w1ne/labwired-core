/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

/* This file should be only included at the end of 'cpu.h'. */

//  These headers require CPUState, cpu_mmu_index etc.
#include "exec-all.h"
#include "cpu_registers.h"

#define SUPPORTS_GUEST_PROFILING

//  Math helpers.
#define ALIGN_DOWN(a, b)     ((a) - ((a) % (b)))
#define ALIGN_UP(a, b)       (IS_MULTIPLE_OF(a, b) ? (a) : (ALIGN_DOWN(a, b) + (b)))
#define DIV_ROUND_UP(a, b)   ((a) / (b) + (IS_MULTIPLE_OF(a, b) ? 0 : 1))
#define IS_MULTIPLE_OF(a, b) (((a) % (b)) == 0)

#define container_of(var, type, base) (type *)(var)

//  We don't need to extract non-specific CPUStates from ARM-specific CPUArchState.
//  It's the same for us.
#define env_cpu(env) env

//  We have these as consts.
#define float16_default_nan(...) float16_default_nan
#define float32_default_nan(...) float32_default_nan
#define float64_default_nan(...) float64_default_nan

#define float16_silence_nan(a, fpst) float16_maybe_silence_nan(a, fpst)
#define float32_silence_nan(a, fpst) float32_maybe_silence_nan(a, fpst)
#define float64_silence_nan(a, fpst) float64_maybe_silence_nan(a, fpst)

//  The define is used to avoid replacing all the 'pc_next' uses.
#define pc_next pc

#define sizeof_field(TYPE, MEMBER) sizeof(((TYPE *)NULL)->MEMBER)

//  TCG function call adjustments.
#define tcg_constant_i32 tcg_const_i32
#define tcg_constant_i64 tcg_const_i64
#if HOST_LONG_BITS == 32
#define tcg_constant_ptr(x) tcg_const_ptr((int32_t)x)
#else
#define tcg_constant_ptr(x) tcg_const_ptr((int64_t)x)
#endif
#define tcg_constant_tl tcg_const_tl

#define ARM_GETPC() ((uintptr_t)GETPC())

//  Keep in line with 'helper_v7.c : bank_number'.
#define BANK_USRSYS 0

//  DISAS_NEXT and DISAS_JUMP (and some unused 'DISAS_') are defined in the common 'exec-all.h'.
#define DISAS_NORETURN 4
#define DISAS_TOO_MANY 5

#define DISAS_TARGET_1  11
#define DISAS_TARGET_2  12
#define DISAS_TARGET_3  13
#define DISAS_TARGET_4  14
#define DISAS_TARGET_5  15
#define DISAS_TARGET_6  16
#define DISAS_TARGET_7  17
#define DISAS_TARGET_8  18
#define DISAS_TARGET_9  19
#define DISAS_TARGET_10 20

//  This is the same as our 'EXCP_WFI'.
#define EXCP_HLT EXCP_WFI

//  Ignore YIELD exception.
#define EXCP_NONE  -1
#define EXCP_YIELD EXCP_NONE

//  Double-check `ldgm` and `stgm` MTE helpers before changing this value.
//  Both these helpers contained static asserts to make sure it's 6.
#define GMID_EL1_BS 6

//  Adjust Memory Operation names.
#define MO_BEUQ MO_BEQ
#define MO_LEUQ MO_LEQ
#define MO_UQ   MO_Q

enum fprounding { FPROUNDING_TIEEVEN, FPROUNDING_POSINF, FPROUNDING_NEGINF, FPROUNDING_ZERO, FPROUNDING_TIEAWAY, FPROUNDING_ODD };

//  Combines MemOp with MMU index.
typedef int MemOpIdx;

//  Defined in 'include/hw/core/cpu.h' (GPL); recreated based on usage.
typedef enum MMUAccessType {
    MMU_DATA_STORE,
    MMU_DATA_LOAD,
} MMUAccessType;

//  Mnemonics that can be used in MRS and MSR instructions.
enum {
    SPSR_ABT,
    SPSR_EL1,  //  SPSR_SVC in AArch32
    SPSR_EL12,
    SPSR_EL2,  //  SPSR_HYP in AArch32
    SPSR_EL3,
    SPSR_FIQ,
    SPSR_IRQ,
    SPSR_UND,
};

enum PMSAv8_FAULT_TYPE {
    ALIGNMENT_FAULT = 0b100001,    //  Access unaligned
    BACKGROUND_FAULT = 0b000000,   //  Not in any region && background not allowed
    PERMISSION_FAULT = 0b001100,   //  Unsufficient permissions
    TRANSLATION_FAULT = 0b000100,  //  Occurs when more than one region contains requested address
    SERROR_FAULT = 0b010001,       //  System error
    DEBUG_FAULT = 0b100010,        //  BKPT instruction, handled with the same flow as prefetch aborts
};

#define FLOAT_TO_INT_FUNC(from_type, to_type)                                                               \
    static inline to_type from_type##_to_##to_type##_scalbn(from_type a, int rmode, int scale STATUS_PARAM) \
    {                                                                                                       \
        from_type scaled = from_type##_scalbn(a, scale STATUS_VAR);                                         \
        return from_type##_to_##to_type(scaled STATUS_VAR);                                                 \
    }

#define INT_TO_FLOAT_FUNC(from_type, to_type)                                                    \
    static inline to_type from_type##_to_##to_type##_scalbn(from_type a, int scale STATUS_PARAM) \
    {                                                                                            \
        to_type conv = from_type##_to_##to_type(a STATUS_VAR);                                   \
        return to_type##_scalbn(conv, scale STATUS_VAR);                                         \
    }

FLOAT_TO_INT_FUNC(float64, int64)
FLOAT_TO_INT_FUNC(float64, uint64)
FLOAT_TO_INT_FUNC(float32, int32)
FLOAT_TO_INT_FUNC(float32, uint32)
FLOAT_TO_INT_FUNC(float64, int32)
FLOAT_TO_INT_FUNC(float64, uint32)
FLOAT_TO_INT_FUNC(float32, int64)
FLOAT_TO_INT_FUNC(float32, uint64)

INT_TO_FLOAT_FUNC(int64, float64)
INT_TO_FLOAT_FUNC(int64, float32)
INT_TO_FLOAT_FUNC(uint64, float64)
INT_TO_FLOAT_FUNC(uint64, float32)
INT_TO_FLOAT_FUNC(int32, float64)
INT_TO_FLOAT_FUNC(int32, float32)
INT_TO_FLOAT_FUNC(uint32, float64)
INT_TO_FLOAT_FUNC(uint32, float32)

//  Provide missing prototypes.
int arm_rmode_to_sf(int rmode);
int bank_number(int mode);
int exception_target_el(CPUARMState *env);
uint64_t mte_check(CPUARMState *env, uint32_t desc, uint64_t ptr, uintptr_t ra);
bool mte_probe(CPUARMState *env, uint32_t desc, uint64_t ptr);
void raise_exception_without_block_end_hooks(CPUARMState *env, uint32_t excp, uint32_t syndrome, uint32_t target_el);
void raise_exception(CPUARMState *env, uint32_t excp, uint32_t syndrome, uint32_t target_el);
void raise_exception_ra(CPUARMState *env, uint32_t excp, uint32_t syndrome, uint32_t target_el, uintptr_t ra);
int r14_bank_number(int mode);

void cpu_init_v8(CPUState *env, uint32_t id);
void cpu_reset_state(CPUState *env);
void cpu_reset_v8_a32(CPUState *env);
void cpu_reset_v8_a64(CPUState *env);
void cpu_reset_vfp(CPUState *env);
void do_interrupt_a32(CPUState *env);
void set_pmsav8_regions_count(CPUState *env, uint32_t el1_regions_count, uint32_t el2_regions_count);
void set_mmu_fault_registers(int access_type, target_ulong address, int fault_type);
void do_interrupt_a64(CPUState *env);
int get_phys_addr(CPUState *env, target_ulong address, int access_type, int mmu_idx, uintptr_t return_address,
                  bool suppress_faults, target_ulong *phys_ptr, int *prot, target_ulong *page_size, int access_size);
int get_phys_addr_v8(CPUState *env, target_ulong address, int access_type, int mmu_idx, uintptr_t return_address,
                     bool suppress_faults, target_ulong *phys_ptr, int *prot, target_ulong *page_size,
                     bool at_instruction_or_cache_maintenance);
int get_phys_addr_v7(CPUState *env, uint32_t address, int access_type, int mmu_idx, target_ulong *phys_ptr, int *prot,
                     target_ulong *page_size, int no_page_fault);
uint32_t pmsav8_number_of_el1_regions(CPUState *env);
uint32_t pmsav8_number_of_el2_regions(CPUState *env);
int process_interrupt_v8a(int interrupt_request, CPUState *env);
uint64_t tlib_crc32(uint64_t crc, const uint8_t *buf, uint32_t length);
uint32_t calculate_crc32c(uint32_t crc32c, const unsigned char *buffer, unsigned int length);

//  TODO: Implement this properly. It's much more complicated for SPSR_EL1 and SPSR_EL2. See:
//  https://developer.arm.com/documentation/ddi0601/2022-09/AArch64-Registers/SPSR-EL1--Saved-Program-Status-Register--EL1-
static inline unsigned int aarch64_banked_spsr_index(int el)
{
    switch(el) {
        case 1:
            return SPSR_EL1;
        case 2:
            return SPSR_EL2;
        case 3:
            return SPSR_EL3;
        default:
            tlib_abortf("aarch64_banked_spsr_index: Invalid el: %d", el);
            __builtin_unreachable();
    }
}

static inline int get_sp_el_idx(CPUState *env)
{
    //  EL0's SP is used if PSTATE_SP (SPSel in AArch64) isn't set.
    return env->pstate & PSTATE_SP ? arm_current_el(env) : 0;
}

static inline void aarch64_save_sp(CPUState *env)
{
    int sp_el_idx = get_sp_el_idx(env);
    env->sp_el[sp_el_idx] = env->xregs[31];
}

static inline void aarch64_restore_sp(CPUState *env)
{
    int sp_el_idx = get_sp_el_idx(env);
    env->xregs[31] = env->sp_el[sp_el_idx];
}

static inline void arm_clear_exclusive(CPUState *env)
{
    env->reserved_address = 0;
}

//  TODO: Calculate effective values for all bits.
//  The returned value is currently valid for all the bits used in tlib:
//        HCR_TGE, HCR_TWE, HCR_TWI, HCR_E2H, HCR_TSC, HCR_AMO,
//        HCR_VSE, HCR_TID0, HCR_TID3, HCR_API, HCR_E2H, HCR_VM
static inline uint64_t arm_hcr_el2_eff(CPUARMState *env)
{
    uint64_t hcr = env->cp15.hcr_el2;
    uint64_t effective_hcr = hcr;

    bool feat_vhe = isar_feature_aa64_vh(&env->arm_core_config.isar);
    bool el2_enabled = arm_is_el2_enabled(env);

    bool tge = hcr & HCR_TGE;
    bool e2h = hcr & HCR_E2H;
    bool dc = hcr & HCR_DC;

    if(tge) {
        effective_hcr &= ~HCR_FB;
        effective_hcr &= ~HCR_TSC;

        if(el2_enabled) {
            effective_hcr &= ~HCR_TID3;
        }
    }

    if(feat_vhe && tge && e2h) {
        effective_hcr &= ~HCR_TWI;
        effective_hcr &= ~HCR_TWE;

        if(el2_enabled) {
            effective_hcr &= ~(HCR_AMO | HCR_FMO | HCR_IMO);
        }

        effective_hcr &= ~HCR_TID0;
        effective_hcr |= HCR_RW;
    } else {
        if(el2_enabled && tge) {
            effective_hcr |= (HCR_AMO | HCR_FMO | HCR_IMO);
        }
    }
    bool amo = effective_hcr & HCR_AMO;

    if(tge || !amo) {
        //  TODO: Should VSE bit be set in the 'else' case? The VSE description
        //        isn't super precise in this matter: "enabled only when the
        //        value of HCR_EL2.{TGE, AMO} is {0, 1}.".
        effective_hcr &= ~HCR_VSE;
    }

    if(is_a64(env) && feat_vhe && e2h && tge) {
        effective_hcr &= ~HCR_DC;
    } else if(dc) {
        effective_hcr |= HCR_VM;
    }

    return effective_hcr;
}

static inline uint64_t arm_sctlr_eff(CPUARMState *env, int el)
{
    uint64_t effective_sctlr = arm_sctlr(env, el);
    uint64_t hcr = arm_hcr_el2_eff(env);

    //  For AArch32 R-profile, SCTLR.M is effectively 0 if HCR.TGE is set. For A-profile,
    //  SCTLR_EL1.M (AArch32: SCTLR.M) is effectively 0 in non-secure state if DC or TGE flag is
    //  set in HCR_EL2 (AArch32: HCR). Neither of these apply to SCTLR_EL2.M (AArch32: HSCTLR.M).
    if(el == 1 && ((hcr & HCR_TGE) || (arm_feature(env, ARM_FEATURE_A) && (hcr & HCR_DC)))) {
        effective_sctlr &= ~SCTLR_M;
    }

    return effective_sctlr;
}

static inline bool arm_is_el3_enabled(CPUState *env)
{
    return arm_feature(env, ARM_FEATURE_EL3);
}

static inline int arm_mmu_idx_to_el(ARMMMUIdx arm_mmu_idx)
{
    //  TODO: M-Profile.
    tlib_assert(arm_mmu_idx & ARM_MMU_IDX_A);

    switch(arm_mmu_idx & ~ARM_MMU_IDX_A_NS) {
        case ARMMMUIdx_SE3:
            return 3;
        case ARMMMUIdx_SE2:
        case ARMMMUIdx_SE20_2:
        case ARMMMUIdx_SE20_2_PAN:
            return 2;
        case ARMMMUIdx_SE10_1:
        case ARMMMUIdx_SE10_1_PAN:
            return 1;
        case ARMMMUIdx_SE10_0:
        case ARMMMUIdx_SE20_0:
            return 0;
        default:
            tlib_abortf("Unsupported arm_mmu_idx: %d", arm_mmu_idx);
            __builtin_unreachable();
    }
}

static inline uint32_t arm_to_core_mmu_idx(ARMMMUIdx arm_mmu_idx)
{
    return arm_mmu_idx & ARM_MMU_IDX_COREIDX_MASK;
}

static inline ARMMMUIdx core_to_arm_mmu_idx(CPUARMState *env, int mmu_idx)
{
    if(arm_feature(env, ARM_FEATURE_M)) {
        return mmu_idx | ARM_MMU_IDX_M;
    }

    return mmu_idx | ARM_MMU_IDX_A;
}

static inline ARMMMUIdx core_to_aa64_mmu_idx(int core_mmu_idx)
{
    return core_mmu_idx | ARM_MMU_IDX_A;
}

static inline void cpu_get_tb_cpu_state(CPUState *env, target_ulong *pc, target_ulong *cs_base, int *flags)
{
    CPUARMTBFlags hflags = env->hflags;
    if(!env->aarch64) {
        DP_TBFLAG_AM32(hflags, THUMB, env->thumb);
        DP_TBFLAG_AM32(hflags, CONDEXEC, env->condexec_bits);
    }

    *pc = CPU_PC(env);
    //  See 'arm_tbflags_from_tb' in 'translate.h'.
    *flags = (int)hflags.flags;
    *cs_base = hflags.flags2;
}

static inline bool cpu_has_work(CPUState *env)
{
    //  clear WFI if waking up condition is met
    env->wfi &= !(is_interrupt_pending(env, CPU_INTERRUPT_HARD));
    env->wfi &= !(is_interrupt_pending(env, CPU_INTERRUPT_FIQ));
    env->wfi &= !(is_interrupt_pending(env, CPU_INTERRUPT_EXITTB));
    env->wfi &= !(is_interrupt_pending(env, CPU_INTERRUPT_VFIQ));
    env->wfi &= !(is_interrupt_pending(env, CPU_INTERRUPT_VIRQ));
    env->wfi &= !(is_interrupt_pending(env, CPU_INTERRUPT_VSERR));
    return !env->wfi;
}

static inline void cpu_pc_from_tb(CPUState *env, TranslationBlock *tb)
{
    if(is_a64(env)) {
        env->pc = tb->pc;
    } else {
        env->regs[15] = tb->pc;
    }
}

static inline bool el2_and_hcr_el2_e2h_set(CPUState *env)
{
    return arm_current_el(env) == 2 && (arm_hcr_el2_eff(env) & HCR_E2H);
}

static inline ARMCoreConfig *env_archcpu(CPUState *env)
{
    return &env->arm_core_config;
}

static inline bool excp_is_internal(uint32_t excp)
{
    switch(excp) {
        case EXCP_EXCEPTION_EXIT:
        case EXCP_SEMIHOST:
            return true;
        default:
            return excp >= 0x10000;  //  All the 0x1000X exceptions are internal.
    }
}

//  The position of the current instruction in the translation block (first is 1).
static inline int get_dcbase_num_insns(DisasContextBase base)
{
    return base.tb->icount;
}

static inline bool are_hcr_e2h_and_tge_set(uint64_t hcr_el2)
{
    uint64_t hcr_e2h_tge = HCR_E2H | HCR_TGE;
    return (hcr_el2 & hcr_e2h_tge) == hcr_e2h_tge;
}

static inline void pstate_set_el(CPUARMState *env, uint32_t el)
{
    //  The function is only valid for AArch64.
    tlib_assert(is_a64(env));
    tlib_assert(el < 4);

    env->pstate = deposit32(env->pstate, 2, 2, el);

    //  Update cached MMUIdx.
    arm_rebuild_hflags(env);

    tlib_on_execution_mode_changed(arm_current_el(env), arm_is_secure(env));
}

static inline void pstate_write_with_sp_change(CPUARMState *env, uint32_t val)
{
    bool modes_differ = (env->pstate & PSTATE_M) != (val & PSTATE_M);
    if(modes_differ) {
        aarch64_save_sp(env);
    }

    pstate_write(env, val);

    if(modes_differ) {
        aarch64_restore_sp(env);

        //  Mostly to update cached MMUIdx.
        arm_rebuild_hflags(env);
    }
}

static inline void pstate_write_masked(CPUARMState *env, uint32_t value, uint32_t mask)
{
    uint32_t new_pstate = (pstate_read(env) & ~mask) | (value & mask);
    pstate_write_with_sp_change(env, new_pstate);
}

static inline bool regime_has_2_ranges(ARMMMUIdx idx)
{
    //  This might be incorrect since it's only based on the names.
    switch(idx) {
        case ARMMMUIdx_E10_0:
        case ARMMMUIdx_E20_0:
        case ARMMMUIdx_E10_1:
        case ARMMMUIdx_E20_2:
        case ARMMMUIdx_E10_1_PAN:
        case ARMMMUIdx_E20_2_PAN:
        case ARMMMUIdx_SE10_0:
        case ARMMMUIdx_SE20_0:
        case ARMMMUIdx_SE10_1:
        case ARMMMUIdx_SE20_2:
        case ARMMMUIdx_SE10_1_PAN:
        case ARMMMUIdx_SE20_2_PAN:
            return true;
        default:
            return false;
    }
}

static inline void set_el_features(CPUState *env, bool el2_enabled, bool el3_enabled)
{
    env->features = deposit64(env->features, ARM_FEATURE_EL2, 1, el2_enabled);
    env->features = deposit64(env->features, ARM_FEATURE_EL3, 1, el3_enabled);
}

static inline uint32_t tb_cflags(TranslationBlock *tb)
{
    return tb->cflags;
}

//  TODO: Port 'tcg_gen_lookup_and_goto_ptr' for more efficient jumps?
//        The upstream function has no arguments.
static inline void tcg_gen_lookup_and_goto_ptr(DisasContext *dc)
{
    gen_exit_tb_no_chaining(dc->base.tb);
}

static inline ARMMMUIdx el_to_arm_mmu_idx(CPUState *env, int el)
{
    ARMMMUIdx idx;
    switch(el) {
        case 0:
            if(are_hcr_e2h_and_tge_set(arm_hcr_el2_eff(env))) {
                idx = ARMMMUIdx_SE20_0;
            } else {
                idx = ARMMMUIdx_SE10_0;
            }
            break;
        case 1:
            idx = pstate_read(env) & PSTATE_PAN ? ARMMMUIdx_SE10_1_PAN : ARMMMUIdx_SE10_1;
            break;
        case 2:
            if(arm_hcr_el2_eff(env) & HCR_E2H) {
                idx = pstate_read(env) & PSTATE_PAN ? ARMMMUIdx_SE20_2_PAN : ARMMMUIdx_SE20_2;
            } else {
                idx = ARMMMUIdx_SE2;
            }
            break;
        case 3:
            idx = ARMMMUIdx_SE3;
            break;
        default:
            tlib_assert_not_reached();
    }

    //  ARMMMUIdx_SE* | ARM_MMU_IDX_A_NS is equivalent to ARMMMUIdx_E*.
    if(!arm_is_secure(env)) {
        idx |= ARM_MMU_IDX_A_NS;
    }
    return idx;
}

static inline int arm_cpu_mode_to_el(CPUState *env, enum arm_cpu_mode mode)
{
    switch(mode) {
        case ARM_CPU_MODE_USR:
            return 0;
        case ARM_CPU_MODE_FIQ:
        case ARM_CPU_MODE_IRQ:
        case ARM_CPU_MODE_SVC:
        case ARM_CPU_MODE_ABT:
        case ARM_CPU_MODE_UND:
        case ARM_CPU_MODE_SYS:
            return 1;
        case ARM_CPU_MODE_HYP:
            return (arm_feature(env, ARM_FEATURE_EL2)) ? 2 : -1;
        case ARM_CPU_MODE_MON:
            return (arm_feature(env, ARM_FEATURE_EL3)) ? 3 : -1;
        default:
            return -1;
    }
}

static inline uint32_t aarch32_cpsr_valid_mask(uint64_t features, const ARMISARegisters *id)
{
    /* The mask is created taking into account many Arm profiles.
     * Check out the following documents.
     *
     * 1. ARM Architecture Reference Manual (ARMv7-A and ARMv7-R edition), B 1.3.1
     *
     *   30  28  26  24  22  20  18  16  14  12  10   8   6   4   2   0
     * ┌─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┐
     * │N│Z│C│V│Q│IT │J│ RAZ*  │  GE   │    IT     │E│A│I│F│T│    M    │
     * └─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┘
     *
     * 2. Arm Architecture Registers (for A-profile Architecture), p. 3577/6056
     *
     *   30  28  26  24  22  20  18  16  14  12  10   8   6   4   2   0
     * ┌─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┐
     * │N│Z│C│V│Q│0 0 0│s│p│d│0│  GE   │0 0 0 0 0 0│E│A│I│F│0│1│   M   │
     * └─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┘
     * Abbreviation:
     *  - s -> SSBS
     *  - p -> PAN
     *  - d -> DIT
     *
     * 3. ARM Cortex-A Series (Programmer’s Guide for ARMv8-A), from 4.5.2
     *
     *   30  28  26  24  22  20  18  16  14  12  10   8   6   4   2   0
     * ┌─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┬─┐
     * │N│Z│C│V│Q│IT │J│     │i│  GE   │    IT     │E│A│I│F│T│M│   M   │
     * └─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┴─┘
     * Abbreviation:
     *  - i -> IL
     */

    uint32_t valid = CPSR_N |   //  Negative result
                     CPSR_Z |   //  Zero result
                     CPSR_C |   //  Carry out
                     CPSR_V |   //  Overflow
                     CPSR_Q |   //  Cumulative saturation
                     CPSR_IL |  //  Illegal Execution State
                     CPSR_GE |  //  Greater than or Equal flags, for the parallel
                                //  addition and subtraction (SIMD) instructions
                     CPSR_E |   //  Endianness execution state (big / little)
                     CPSR_A |   //  Asynchronous abort mask
                     CPSR_I |   //  IRQ mask
                     CPSR_F |   //  FIQ mask
                     CPSR_T |   //  Thumb execution state
                     CPSR_M;    //  Mode

    if(features & (UINT64_C(1) << ARM_FEATURE_THUMB2)) {
        valid |= CPSR_IT;  //  If-Then execution state bits for the Thumb IT (If-Then) instruction
    }

    if(isar_feature_aa32_jazelle(id)) {
        valid |= CPSR_J;  //  Jazelle
    }

    if(isar_feature_aa32_ssbs(id)) {
        valid |= CPSR_SSBS;  //  Speculative Store Bypass Safe
    }

    if(isar_feature_aa32_pan(id)) {
        valid |= CPSR_PAN;  //  Privileged Access Never
    }

    if(isar_feature_aa32_dit(id)) {
        valid |= CPSR_DIT;  //  Data Independent Timing
    }

    return valid;
}

static inline enum arm_cpu_mode arm_get_highest_cpu_mode(CPUState *env)
{
    if(arm_feature(env, ARM_FEATURE_EL3)) {
        return ARM_CPU_MODE_MON;
    } else if(arm_feature(env, ARM_FEATURE_EL2)) {
        return ARM_CPU_MODE_HYP;
    } else {
        return ARM_CPU_MODE_SVC;
    }
}

static inline uint32_t address_translation_el(CPUState *env, uint32_t el)
{
    if(el != 0) {
        return el;
    }

    if(arm_is_el2_enabled(env) && are_hcr_e2h_and_tge_set(arm_hcr_el2_eff(env))) {
        return 2;
    } else {
        return 1;
    }
}

static inline uint64_t arm_tcr(CPUState *env, int el)
{
    tlib_assert(el >= 0 && el <= 3);
    return env->cp15.tcr_el[address_translation_el(env, el)];
}

static inline uint64_t arm_ttbr0(CPUState *env, int el)
{
    tlib_assert(el >= 0 && el <= 3);
    return env->cp15.ttbr0_el[address_translation_el(env, el)];
}

static inline uint64_t arm_ttbr1(CPUState *env, int el)
{
    tlib_assert(el >= 0 && el <= 3);
    return env->cp15.ttbr1_el[address_translation_el(env, el)];
}

static inline void find_pending_irq_if_primask_unset(CPUState *env)
{
#ifdef TARGET_PROTO_ARM_M
    //  There are not enough helpers to emulate M-profile core with this lib
    tlib_abort("ARM-M mode is not supported");
#endif
}

static inline int get_fp_exc_el(CPUARMState *env, int el)
{
    uint64_t hcr_el2_e2h = arm_hcr_el2_eff(env) & HCR_E2H;
    uint64_t hcr_el2_tge = arm_hcr_el2_eff(env) & HCR_TGE;

    //  Mainly based on CPACR_EL1's Configurations section and FPEN bits
    //  (ARM Architecture Reference Manual for A-Profile architecture D17.2.30).
    if(!hcr_el2_e2h || !hcr_el2_tge) {
        int fpen = FIELD_EX64(env->cp15.cpacr_el1, CPACR_EL1, FPEN);
        switch(fpen) {
            case 0b01:
                if(el > 0) {
                    break;
                }
            /* fallthrough */
            case 0b00:
            case 0b10:
                if(!arm_el_is_aa64(env, 3) && arm_is_secure(env)) {
                    return 3;
                }
                if(el <= 1) {
                    return 1;
                }
                break;
                /* 0b11 - no trap */
        }
    }

    if(el <= 2) {
        if(hcr_el2_e2h) {
            switch(FIELD_EX64(env->cp15.cptr_el[2], CPTR_EL2, FPEN)) {
                case 0b01:
                    if(el > 0 || !hcr_el2_tge) {
                        break;
                    }
                /* fallthrough */
                case 0b00:
                case 0b10:
                    return 2;
                    /* 0b11 - no trap */
            }
        } else if(arm_feature(env, ARM_FEATURE_EL2)) {
            if(FIELD_EX64(env->cp15.cptr_el[2], CPTR_EL2, TFP)) {
                return 2;
            }
        }
    }

    if(FIELD_EX64(env->cp15.cptr_el[3], CPTR_EL3, TFP)) {
        return 3;
    }
    return 0;
}
