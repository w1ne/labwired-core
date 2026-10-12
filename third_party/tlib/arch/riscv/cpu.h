#pragma once

#include "bit_helper.h"
#include "cpu-defs.h"
#include "softfloat.h"
#include "softfloat-2.h"
#include "host-utils.h"

#define SUPPORTS_GUEST_PROFILING
#define SUPPORTS_HST_ATOMICS

#define TARGET_PAGE_BITS 12 /* 4 KiB Pages */
#if TARGET_LONG_BITS == 64
#define TARGET_RISCV64
#define TARGET_PHYS_ADDR_SPACE_BITS 56
#elif TARGET_LONG_BITS == 32
#define TARGET_RISCV32
#define TARGET_PHYS_ADDR_SPACE_BITS 34
#else
#error "Target arch can be only 32-bit or 64-bit."
#endif

#include "cpu_bits.h"
#include "cpu_registers.h"

#define RV(x) ((target_ulong)1 << (x - 'A'))

#define NB_MMU_MODES 4

#define MAX_RISCV_PMPS (64)

//  In MISA register the extensions are encoded on bits [25:0] (see:
//  https://five-embeddev.com/riscv-isa-manual/latest/machine.html), but because these additional features are not there
//  RISCV_ADDITIONAL_FEATURE_OFFSET allows to show that they are unrelated to MISA.
#define RISCV_ADDITIONAL_FEATURE_OFFSET 26

#define get_field(reg, mask) (((reg) & (target_ulong)(mask)) / ((mask) & ~((mask) << 1)))
#define set_field(reg, mask, val) \
    (((reg) & ~(target_ulong)(mask)) | (((target_ulong)(val) * ((mask) & ~((mask) << 1))) & (target_ulong)(mask)))

#define assert(x)                                                               \
    {                                                                           \
        if(!(x))                                                                \
            tlib_abortf("Assert not met in %s:%d: %s", __FILE__, __LINE__, #x); \
    }                                                                           \
    while(0)

typedef struct custom_instruction_descriptor_t {
    uint64_t id;
    uint64_t length;
    uint64_t mask;
    uint64_t pattern;
} custom_instruction_descriptor_t;
#define CPU_CUSTOM_INSTRUCTIONS_LIMIT 256

#define MAX_CSR_ID    0xFFF
#define CSRS_PER_SLOT 64
#define CSRS_SLOTS    (MAX_CSR_ID + 1) / CSRS_PER_SLOT

#define VLEN_MAX (1 << 16)

#define RISCV_CPU_INTERRUPT_CLIC CPU_INTERRUPT_TGT_EXT_0
//  Intended to be used for any core specific custom interrupts
#define RISCV_CPU_INTERRUPT_CUSTOM CPU_INTERRUPT_TGT_EXT_1

typedef struct DisasContext {
    struct DisasContextBase base;
    uint64_t opcode;
    target_ulong npc;
} DisasContext;

typedef struct CPUState CPUState;

#include "cpu-common.h"
#include "pmp.h"

//  +---------------------------------------+
//  | ALL FIELDS WHICH STATE MUST BE STORED |
//  | DURING SERIALIZATION SHOULD BE PLACED |
//  | BEFORE >CPU_COMMON< SECTION.          |
//  +---------------------------------------+
struct CPUState {
    target_ulong gpr[32];
    uint64_t fpr[32]; /* assume both F and D extensions */
    uint8_t vr[32 * (VLEN_MAX / 8)];
    target_ulong pc;
    target_ulong opcode;

    uint32_t frm;
    uint32_t fflags;

    target_ulong badaddr;

    target_ulong priv;

    target_ulong misa;
    target_ulong misa_mask;
    target_ulong mstatus;

    target_ulong mhartid;

    pthread_mutex_t mip_lock;
    target_ulong mip;
    target_ulong mie;
    target_ulong mideleg;

    target_ulong sptbr; /* until: priv-1.9.1;  replaced by satp */
    target_ulong medeleg;

    target_ulong stvec;
    target_ulong stvt;       /* unratified as of 2024-06; ssclic extension */
    target_ulong sintthresh; /* unratified as of 2024-06; ssclic extension */
    target_ulong sepc;
    target_ulong scause;
    target_ulong stval; /* renamed from sbadaddr since: priv-1.10.0 */
    target_ulong satp;  /* since: priv-1.10.0 */
    target_ulong sedeleg;
    target_ulong sideleg;

    target_ulong mtvec;
    target_ulong mtvt;       /* unratified as of 2024-06; smclic extension */
    target_ulong mintthresh; /* unratified as of 2024-06; smclic extension */
    target_ulong mepc;
    target_ulong mcause;
    target_ulong mtval; /*  renamed from mbadaddr since: priv-1.10.0 */

    uint32_t mucounteren;    /* until 1.10.0 */
    uint32_t mscounteren;    /* until 1.10.0 */
    target_ulong scounteren; /* since: priv-1.10.0 */
    target_ulong mcounteren; /* since: priv-1.10.0 */
    uint32_t mcountinhibit;  /* since: priv-1.11 */

    target_ulong sscratch;
    target_ulong mscratch;
    target_ulong mintstatus; /* unratified as of 2024-06; smclic extension */

    target_ulong vstart;
    target_ulong vxsat;
    target_ulong vxrm;
    target_ulong vcsr; /* This field is a cached view of VXRM and VXSAT. Remember to update them in sync. */
    target_ulong vl;
    target_ulong vtype;
    target_ulong vlenb;

    target_ulong prev_sp;

    target_ulong menvcfg;
    target_ulong menvcfgh;
    target_ulong mseccfg;
    target_ulong mseccfgh;

    target_ulong jvt;

    /* Vector shadow state */
    target_ulong elen;
    target_ulong vlmax;

    target_ulong vsew;
    target_ulong vlmul;
    float vflmul;
    target_ulong vill;
    target_ulong vta;
    target_ulong vma;

    /* temporary htif regs */
    uint64_t mfromhost;
    uint64_t mtohost;
    uint64_t timecmp;

    /* physical memory protection */
    pmp_table_t pmp_state;
    target_ulong pmp_addr_mask;
    bool use_external_pmp;

    float_status fp_status;

    uint64_t mcycle_snapshot_offset;
    uint64_t mcycle_snapshot;

    uint64_t minstret_snapshot_offset;
    uint64_t minstret_snapshot;

    /* non maskable interrupts */
    uint32_t nmi_pending;
    target_ulong nmi_address;
    uint32_t nmi_length;
    target_ulong nmi_mcause[32];

    int privilege_architecture;

    int32_t custom_instructions_count;
    custom_instruction_descriptor_t custom_instructions[CPU_CUSTOM_INSTRUCTIONS_LIMIT];

    //  bitmap keeping information about CSRs that have custom external implementation
    uint64_t custom_csrs[CSRS_SLOTS];

    //  bitmap holding installed custom local interrupts, encoded the same way as the bits in `mie` and `mip`.
    target_ulong custom_interrupts;
    //  bitmap holding which installed custom local interrupts can be triggered by writing to `mip` and `sip`
    target_ulong mip_triggered_custom_interrupts;
    target_ulong sip_triggered_custom_interrupts;

    /*
       Supported CSR validation levels:
     * 0 - (CSR_VALIDATION_NONE): no validation
     * 1 - (CSR_VALIDATION_PRIV): privilege level validation only
     * 2 - (CSR_VALIDATION_FULL): full validation - privilege level and read/write bit validation

     * Illegal Instruction Exception* is generated when validation fails

       Levels are defined in `cpu_bits.h`
     */
    int32_t csr_validation_level;

    /* flags indicating extensions from which instructions
       that are *not* enabled for this CPU should *not* be logged as errors;

       this is useful when some instructions are `software-emulated`,
       i.e., the ILLEGAL INSTRUCTION exception is generated and handled by the software */
    target_ulong silenced_extensions;

    uint32_t additional_extensions;

    /* since priv-1.11.0 pmp grain size must be the same across all pmp regions */
    int32_t pmp_napot_grain;

    uint32_t pmp_entry_count;

    /* Supported modes:
     * 0 (INTERRUPT_MODE_AUTO) - check mtvec's LSB to detect mode: 0->direct, 1->vectored, 3->clic
     * 1 (INTERRUPT_MODE_DIRECT) - all exceptions set pc to mtvec's BASE
     * 2 (INTERRUPT_MODE_VECTORED) - asynchronous interrupts set pc to mtvec's BASE + 4 * cause
     */
    int32_t interrupt_mode;

    int32_t clic_interrupt_pending;
    uint32_t clic_interrupt_vectored;
    uint32_t clic_interrupt_level;
    uint32_t clic_interrupt_priv;

    bool is_pre_stack_access_hook_enabled;

    CPU_COMMON

    int8_t are_post_gpr_access_hooks_enabled;
    uint32_t post_gpr_access_hook_mask;
};

void riscv_set_mode(CPUState *env, target_ulong newpriv);

void helper_raise_exception(CPUState *env, uint32_t exception);
void helper_raise_illegal_instruction(CPUState *env);

int cpu_handle_mmu_fault(CPUState *cpu, target_ulong address, int rw, int mmu_idx, int access_width, int no_page_fault,
                         target_phys_addr_t *paddr);

static inline int cpu_mmu_index(CPUState *env)
{
    return env->priv;
}

int riscv_cpu_hw_interrupts_pending(CPUState *env);

#include "cpu-all.h"
#include "exec-all.h"

static inline void cpu_get_tb_cpu_state(CPUState *env, target_ulong *pc, target_ulong *cs_base, int *flags)
{
    *pc = env->pc;
    *cs_base = 0;
    *flags = cpu_mmu_index(env);
}

static inline bool cpu_has_work(CPUState *env)
{
    //  clear WFI if waking up condition is met
    //  this CLIC interrupt check is a bit overly eager, but it's faster than checking all the conditions
    env->wfi &= !((cpu->mip & cpu->mie) || cpu->clic_interrupt_pending != EXCP_NONE);
    return !env->wfi;
}

static inline int riscv_mstatus_fs(CPUState *env)
{
    return env->mstatus & MSTATUS_FS;
}

static inline void raise_exception(CPUState *env, uint32_t exception)
{
    env->exception_index = exception;
}

//  It must be always inlined, because GETPC() macro must be called in the context of function that is directly invoked by
//  generated code.
static inline void __attribute__((always_inline, __noreturn__)) raise_exception_and_sync_pc(CPUState *env, uint32_t exception)
{
    uintptr_t pc = (uintptr_t)GETPC();
    raise_exception(env, exception);
    cpu_loop_exit_restore(env, pc, 1);
}

void cpu_set_nmi(CPUState *env, int number, target_ulong mcause);

void cpu_reset_nmi(CPUState *env, int number);

void csr_write_helper(CPUState *env, target_ulong val_to_write, target_ulong csrno);

void do_nmi(CPUState *env);

static inline void cpu_pc_from_tb(CPUState *cs, TranslationBlock *tb)
{
    cs->pc = tb->pc;
}

enum riscv_feature {
    RISCV_FEATURE_RVI = RV('I'),
    RISCV_FEATURE_RVM = RV('M'),
    RISCV_FEATURE_RVA = RV('A'),
    RISCV_FEATURE_RVF = RV('F'),
    RISCV_FEATURE_RVD = RV('D'),
    RISCV_FEATURE_RVC = RV('C'),
    RISCV_FEATURE_RVS = RV('S'),
    RISCV_FEATURE_RVU = RV('U'),
    RISCV_FEATURE_RVV = RV('V'),
    RISCV_FEATURE_RVE = RV('E'),
};

enum riscv_additional_feature {
    RISCV_FEATURE_ZBA = 0,
    RISCV_FEATURE_ZBB = 1,
    RISCV_FEATURE_ZBC = 2,
    RISCV_FEATURE_ZBS = 3,
    RISCV_FEATURE_ZICSR = 4,
    RISCV_FEATURE_ZIFENCEI = 5,
    RISCV_FEATURE_ZFH = 6,
    RISCV_FEATURE_ZVFH = 7,
    RISCV_FEATURE_SMEPMP = 8,
    RISCV_FEATURE_ZVE32X = 9,
    RISCV_FEATURE_ZVE32F = 10,
    RISCV_FEATURE_ZVE64X = 11,
    RISCV_FEATURE_ZVE64F = 12,
    RISCV_FEATURE_ZVE64D = 13,
    RISCV_FEATURE_ZACAS = 14,
    RISCV_FEATURE_SSCOFPMF = 15,
    RISCV_FEATURE_ZBKB = 16,
    RISCV_FEATURE_ZCB = 17,
    RISCV_FEATURE_ZCMP = 18,
    RISCV_FEATURE_ZCMT = 19,
    RISCV_FEATURE_ZICOND = 20,
    RISCV_FEATURE_ONE_HIGHER_THAN_HIGHEST_ADDITIONAL
};

enum privilege_architecture {
    RISCV_PRIV1_09,
    RISCV_PRIV1_10,
    RISCV_PRIV1_11,
    RISCV_PRIV1_12,
    //  For features that are not yet part of ratified privileged architecture,
    //  use RISCV_PRIV_UNRATIFIED.
    //  Replace with an actual version once it becomes a part of ratified spec.
    //  KEEP LAST
    RISCV_PRIV_UNRATIFIED
};

//  The enum encodes the fmt field of opcode
enum riscv_floating_point_precision {
    RISCV_SINGLE_PRECISION = 0b00,
    RISCV_DOUBLE_PRECISION = 0b01,
    RISCV_HALF_PRECISION = 0b10,
    RISCV_QUAD_PRECISION = 0b11
};

static inline int riscv_has_ext(CPUState *env, target_ulong ext)
{
    return (env->misa & ext) != 0;
}

static inline int riscv_has_additional_ext(CPUState *env, enum riscv_additional_feature extension)
{
    return !!(env->additional_extensions & (1U << extension));
}

static inline int riscv_silent_ext(CPUState *env, target_ulong ext)
{
    return (env->silenced_extensions & ext) != 0;
}

static inline int riscv_features_to_string(uint32_t features, char *buffer, int size)
{
    //  features are encoded on the first 26 bits
    //  bit #0: 'A', bit #1: 'B', ..., bit #25: 'Z'
    int i, pos = 0;
    for(i = 0; i < 26 && pos < size; i++) {
        if(features & (1 << i)) {
            buffer[pos++] = 'A' + i;
        }
    }
    return pos;
}

static void __attribute__((__noreturn__)) log_disabled_extension_and_raise_exception(CPUState *env, uintptr_t host_pc,
                                                                                     enum riscv_feature ext, const char *message)
{
    cpu_restore_state(env, (void *)host_pc);
    if(!riscv_silent_ext(cpu, ext)) {
        target_ulong guest_pc = env->pc;
        char letter = 0;
        riscv_features_to_string(ext, &letter, 1);
        if(message == NULL) {
            tlib_printf(LOG_LEVEL_ERROR, "PC: 0x%llx, RISC-V '%c' instruction set is not enabled for this CPU! ", guest_pc,
                        letter);
        } else {
            tlib_printf(LOG_LEVEL_ERROR, "PC: 0x%llx, RISC-V '%c' instruction set is not enabled for this CPU! %s", guest_pc,
                        letter, message);
        }
    }
    raise_exception(env, RISCV_EXCP_ILLEGAL_INST);
    cpu_loop_exit(env);
}

static inline void __attribute__((always_inline)) ensure_vector_embedded_extension_or_raise_exception(CPUState *env)
{
    //  Check if the most basic extension is supported.
    if(riscv_has_additional_ext(env, RISCV_FEATURE_ZVE32X)) {
        return;
    }

    log_disabled_extension_and_raise_exception(env, (uintptr_t)GETPC(), RISCV_FEATURE_RVV, NULL);
}

static inline void __attribute__((always_inline)) ensure_vector_embedded_extension_for_eew_or_raise_exception(CPUState *env,
                                                                                                              target_ulong eew)
{
    uintptr_t pc = (uintptr_t)GETPC();

    //  Assume there is no EEW larger than 64.
    if(riscv_has_additional_ext(env, RISCV_FEATURE_ZVE64X)) {
        return;
    }

    if(riscv_has_additional_ext(env, RISCV_FEATURE_ZVE32X)) {
        if(eew < 64) {
            return;
        }
        log_disabled_extension_and_raise_exception(env, pc, RISCV_FEATURE_RVV, "EEW is too large for the Zve32x extension");
    } else {
        log_disabled_extension_and_raise_exception(env, pc, RISCV_FEATURE_RVV, NULL);
    }
}

static inline void __attribute__((always_inline)) ensure_vector_float_embedded_extension_or_raise_exception(CPUState *env,
                                                                                                            target_ulong eew)
{
    uintptr_t pc = (uintptr_t)GETPC();

    if(eew == 32) {
        if(riscv_has_additional_ext(env, RISCV_FEATURE_ZVE64F) || riscv_has_additional_ext(env, RISCV_FEATURE_ZVE32F)) {
            return;
        }
        log_disabled_extension_and_raise_exception(
            env, pc, RISCV_FEATURE_RVV, "Zve64f or Zve32f is required for single precision floating point vector operations");
    }
    if(eew == 16) {
        if(riscv_has_additional_ext(env, RISCV_FEATURE_ZVFH)) {
            return;
        }
        log_disabled_extension_and_raise_exception(env, pc, RISCV_FEATURE_RVV,
                                                   "Zvfh is required for half precision floating point vector operations");
    }
    if(eew == 64) {
        if(riscv_has_additional_ext(env, RISCV_FEATURE_ZVE64D)) {
            return;
        }
        log_disabled_extension_and_raise_exception(env, pc, RISCV_FEATURE_RVV,
                                                   "Zve64d is required for double precision floating point vector operations");
    }

    char eww_unsupported_message[64];
    if(snprintf(eww_unsupported_message, 63, "EEW (%d) isn't supported for vector floating point extensions", (unsigned int)eew) <
       0) {
        tlib_abort("Unable to print log about unsupported EEW!");
    }
    log_disabled_extension_and_raise_exception(env, pc, RISCV_FEATURE_RVV, eww_unsupported_message);
}

static inline void mark_fs_dirty()
{
    env->mstatus |= (MSTATUS_FS | MSTATUS_XS);
}

static inline bool cpu_in_clic_mode(CPUState *env)
{
    return get_field(env->mtvec, MTVEC_MODE) == MTVEC_MODE_CLIC;
}

static inline void set_default_mstatus()
{
    if(riscv_has_ext(env, RISCV_FEATURE_RVD) || riscv_has_ext(env, RISCV_FEATURE_RVF)) {
        env->mstatus = (MSTATUS_FS_INITIAL | MSTATUS_XS_INITIAL);
    } else {
        env->mstatus = 0;
    }
    env->mstatus = set_field(env->mstatus, MSTATUS_MPP, PRV_M);
    if(cpu_in_clic_mode(env)) {
        /* MPP and MPIE are mirrored */
        env->mcause = set_field(env->mcause, MCAUSE_MPP, PRV_M);
    }
}

static inline int supported_fpu_extensions_count(CPUState *env)
{
    return !!riscv_has_ext(env, RISCV_FEATURE_RVF) + !!riscv_has_ext(env, RISCV_FEATURE_RVD);
}

static inline bool is_unboxing_needed(enum riscv_floating_point_precision float_precision, CPUState *env)
{
    return float_precision != RISCV_DOUBLE_PRECISION && supported_fpu_extensions_count(env) != 1;
}

static inline uint64_t get_float_mask(enum riscv_floating_point_precision float_precision)
{
    switch(float_precision) {
        case RISCV_DOUBLE_PRECISION:
            return UINT64_MAX;
        case RISCV_SINGLE_PRECISION:
            return UINT32_MAX;
        case RISCV_HALF_PRECISION:
            return UINT16_MAX;
        default:
            //  Should never happen.
            tlib_abortf("Unsupported floating point precision: %d. Can't provide a mask for it.", float_precision);
            return 0;
    }
}

static inline float64 get_float_default_nan(enum riscv_floating_point_precision float_precision)
{
    switch(float_precision) {
        case RISCV_DOUBLE_PRECISION:
            return float64_default_nan;
        case RISCV_SINGLE_PRECISION:
            return float32_default_nan;
        case RISCV_HALF_PRECISION:
            return float16_default_nan;
        default:
            //  Should never happen.
            tlib_abortf("Unsupported floating point precision: %d. Can't provide a default NaN for it.", float_precision);
            return 0;
    }
}

static inline int64_t get_float_sign_mask(enum riscv_floating_point_precision float_precision)
{
    switch(float_precision) {
        case RISCV_DOUBLE_PRECISION:
            return INT64_MIN;
        case RISCV_SINGLE_PRECISION:
            return INT32_MIN;
        case RISCV_HALF_PRECISION:
            return INT16_MIN;
        default:
            //  Should never happen.
            tlib_abortf("Unsupported floating point precision: %d. Can't provide a default NaN for it.", float_precision);
            return 0;
    }
}

static inline float64 unbox_float(enum riscv_floating_point_precision float_precision, CPUState *env, float64 value)
{
    if(!is_unboxing_needed(float_precision, env)) {
        return value;
    }

    bool is_box_valid = ((uint64_t)value | get_float_mask(float_precision)) == UINT64_MAX;
    return is_box_valid ? value : get_float_default_nan(float_precision);
}

static inline float64 box_float(enum riscv_floating_point_precision float_precision, float64 value)
{
    return value | ~get_float_mask(float_precision);
}

static inline void gen_unbox_float(enum riscv_floating_point_precision float_precision, CPUState *env, TCGv_i64 destination,
                                   TCGv_i64 source)
{
    //  The function consists of more than one basic block, because there is a branch inside it.
    //  The destination variable must be at least local temporary.
    tcg_gen_mov_i64(destination, source);
    if(!is_unboxing_needed(float_precision, env)) {
        return;
    }

    int valid_box = gen_new_label();
    TCGv_i64 temp = tcg_temp_new_i64();
    tcg_gen_ori_i64(temp, source, get_float_mask(float_precision));
    tcg_gen_brcondi_i64(TCG_COND_EQ, temp, UINT64_MAX, valid_box);
    tcg_gen_movi_i64(destination, get_float_default_nan(float_precision));
    gen_set_label(valid_box);
    tcg_temp_free_i64(temp);
}

static inline void gen_box_float(enum riscv_floating_point_precision float_precision, TCGv_i64 value)
{
    tcg_gen_ori_i64(value, value, ~get_float_mask(float_precision));
}

#define GET_VTYPE_VLMUL(inst) extract32(inst, 0, 3)
#define GET_VTYPE_VSEW(inst)  extract32(inst, 3, 3)
#define GET_VTYPE_VTA(inst)   extract32(inst, 6, 1)
#define GET_VTYPE_VMA(inst)   extract32(inst, 7, 1)

//  Vector registers are defined as contiguous segments of vlenb bytes.
#define V(x)  (env->vr + (x) * env->vlenb)
#define SEW() GET_VTYPE_VSEW(env->vtype)
/* This returns the emul for the destination endcoded just as the vlmul field, the eew (for the destination) must be encoded just
like the SEW field.
// Effectively this just adjusts the emul to the resulting element width change in case of narrowing/widening instructions
// and should not be used in other cases */
#define EMUL(eew) (((int8_t)(env->vlmul & 0x4 ? env->vlmul | 0xf8 : env->vlmul) + (eew) - SEW()) & 0x7)

#define RESERVED_EMUL 0x4

//  if EMUL >= 1 then n has to be divisible by EMUL
//  The emul value here is encoded the same way the vlmul field is
#define V_IDX_INVALID_EMUL(n, emul) ((emul) < 0x4 && ((n) & ((1 << (emul)) - 1)) != 0)
#define V_IDX_INVALID_EEW(n, eew)   V_IDX_INVALID_EMUL(n, EMUL(eew))
#define V_IDX_INVALID(n)            V_IDX_INVALID_EMUL(n, env->vlmul)
#define V_INVALID_NF(vd, nf, emul)  (((emul) & 0x4) != 0 && (((nf) << (emul)) >= 8 || ((vd) + ((nf) << (emul))) >= 32))
