/*
 * ARM virtual CPU header
 *
 *  Copyright (c) 2003 Fabrice Bellard
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see <http://www.gnu.org/licenses/>.
 */
#pragma once

#include <stdbool.h>
#include <stdlib.h>
#include "cpu-defs.h"
#include "bit_helper.h"  // extract32
#include "tightly_coupled_memory.h"
#include "ttable.h"
#include "pmu.h"
#include "cpu_common.h"

#include "softfloat-2.h"
#include "arch_callbacks.h"

#define SUPPORTS_GUEST_PROFILING

#if TARGET_LONG_BITS == 32
#define TARGET_ARM32
#elif TARGET_LONG_BITS == 64
#define TARGET_ARM64
#else
#error "Target arch can be only 32-bit or 64-bit"
#endif

#include "cpu_registers.h"

#define EXCP_UDEF           1 /* undefined instruction */
#define EXCP_SWI            2 /* software interrupt */
#define EXCP_PREFETCH_ABORT 3
#define EXCP_DATA_ABORT     4
#define EXCP_IRQ            5
#define EXCP_FIQ            6
#define EXCP_BKPT           7
#define EXCP_KERNEL_TRAP    9 /* Jumped to kernel code page.  */
#define EXCP_STREX          10
#define EXCP_NOCP           17 /* NOCP usage fault */
#define EXCP_INVSTATE       18 /* INVSTATE usage fault */
#define EXCP_SECURE         19 /* TrustZone Secure fault */
#define EXCP_DIV_0          20 /* Integer divide by zero fault */
#define EXCP_BUS_FAULT      21 /* Synchronous, precise data BusFault */

#define ARMV7M_EXCP_RESET    1
#define ARMV7M_EXCP_NMI      2
#define ARMV7M_EXCP_HARD     3
#define ARMV7M_EXCP_MEM      4
#define ARMV7M_EXCP_BUS      5
#define ARMV7M_EXCP_USAGE    6
#define ARMV7M_EXCP_SECURE   7
#define ARMV7M_EXCP_SVC      11
#define ARMV7M_EXCP_DEBUG    12
#define ARMV7M_EXCP_PENDSV   14
#define ARMV7M_EXCP_SYSTICK  15
#define ARMV7M_EXCP_HARDIRQ0 16 /* Hardware IRQ0. Any exceptions above this one are also hard IRQs */
#define ARMV7M_LOCKUP_PC     0xeffffffe

typedef enum {
    V7M_EXCEPTION_PHASE_NONE,
    V7M_EXCEPTION_PHASE_STACKING,
    V7M_EXCEPTION_PHASE_VECTOR_READ,
    V7M_EXCEPTION_PHASE_UNSTACKING,
    V7M_EXCEPTION_PHASE_LAZY_FP,
} V7MExceptionPhase;

/* Default value for LTPSIZE field in FPSCR. It indicates that tail predication is switched off */
#define LTPSIZE_PREDICATION_DISABLED 4

/* For banked exceptions, we store information what Security mode they target
 * in a specific bit of exception number (higher than max supported exceptions).
 * This is just a mechanism on our side, from CPUs perspective, banked exceptions have the same
 * exception numbers as without Security Extensions (TrustZone).
 * In effect, we have extra exceptions now. */
#define BANKED_SECURE_EXCP_BIT (1 << 30)
#define BANKED_SECURE_EXCP(x)  (x | BANKED_SECURE_EXCP_BIT)

/* MemManage Fault : bits 0:7 of CFSR */
#define MEM_FAULT_MMARVALID 1 << 7
#define MEM_FAULT_MLSPERR   1 << 5
#define MEM_FAULT_MSTKERR   1 << 4
#define MEM_FAULT_MUNSTKERR 1 << 3
#define MEM_FAULT_DACCVIOL  1 << 1
#define MEM_FAULT_IACCVIOL  1 << 0
/* BusFault : bits 8:15 of CFSR */
#define BUS_FAULT_OFFSET      8
#define BUS_FAULT_BFARVALID   ((1 << 7) << BUS_FAULT_OFFSET)
#define BUS_FAULT_LSPERR      ((1 << 5) << BUS_FAULT_OFFSET)
#define BUS_FAULT_STKERR      ((1 << 4) << BUS_FAULT_OFFSET)
#define BUS_FAULT_UNSTKERR    ((1 << 3) << BUS_FAULT_OFFSET)
#define BUS_FAULT_PRECISERR   ((1 << 1) << BUS_FAULT_OFFSET)
#define BUS_FAULT_STATUS_MASK (0xff << BUS_FAULT_OFFSET)
/* Usage Fault : bits 16-31 of CFSR */
#define USAGE_FAULT_OFFSET     16
#define USAGE_FAULT_DIVBYZERO  (1 << 9) << USAGE_FAULT_OFFSET
#define USAGE_FAULT_UNALIGNED  (1 << 8) << USAGE_FAULT_OFFSET
#define USAGE_FAULT_NOCP       (1 << 3) << USAGE_FAULT_OFFSET
#define USAGE_FAULT_INVPC      (1 << 2) << USAGE_FAULT_OFFSET
#define USAGE_FAULT_INVSTATE   (1 << 1) << USAGE_FAULT_OFFSET
#define USAGE_FAULT_UNDEFINSTR (1 << 0) << USAGE_FAULT_OFFSET

/* Secure Fault */
#define SECURE_FAULT_LSERR     (1 << 7)
#define SECURE_FAULT_SFARVALID (1 << 6)
#define SECURE_FAULT_LSPERR    (1 << 5)
#define SECURE_FAULT_INVTRAN   (1 << 4)
#define SECURE_FAULT_AUVIOL    (1 << 3)
#define SECURE_FAULT_INVER     (1 << 2)
#define SECURE_FAULT_INVIS     (1 << 1)
#define SECURE_FAULT_INVEP     (1 << 0)

//  256 is a hard limit based on width of their respective region number fields in TT instructions.
#define MAX_MPU_REGIONS  256
#define MAX_SAU_REGIONS  256
#define MAX_IDAU_REGIONS 256

#define MAX_IMPL_DEF_ATTRIBUTION_EXEMPTIONS 256

#define MPU_SIZE_FIELD_MASK                 0x3E
#define MPU_REGION_ENABLED_BIT              0x1
#define MPU_SIZE_AND_ENABLE_FIELD_MASK      (MPU_SIZE_FIELD_MASK | MPU_REGION_ENABLED_BIT)
#define MPU_NEVER_EXECUTE_BIT               0x1000
#define MPU_PERMISSION_FIELD_MASK           0x700
#define MPU_SUBREGION_DISABLE_FIELD_MASK    0xFF00
#define MPU_TYPE_DREGION_FIELD_OFFSET       8
#define MPU_TYPE_DREGION_FIELD_MASK         (0xFF << MPU_TYPE_DREGION_FIELD_OFFSET)
#define MPU_SUBREGION_DISABLE_FIELD_OFFSET  8
#define MPU_FAULT_STATUS_BITS_FIELD_MASK    0x40f
#define MPU_FAULT_STATUS_WRITE_FIELD_OFFSET 11
#define MPU_FAULT_STATUS_WRITE_FIELD_MASK   (1 << 11)

#define BACKGROUND_FAULT_STATUS_BITS 0b0000
#define PERMISSION_FAULT_STATUS_BITS 0b1101

#define FNC_RETURN 0xFEFFFFFF
//  Bit[0] - "SFTC" - is used to check if FPU was enabled when taking exception (corresponds to: ARM_EXC_RETURN_NFPCA)
#define INTEGRITY_SIGN 0xFEFA125A

typedef struct DisasContext {
    DisasContextBase base;
    /* Nonzero if this instruction has been conditionally skipped.  */
    int condjmp;
    /* The label that will be jumped to when the instruction is skipped.  */
    int condlabel;
    /* Thumb-2 condtional execution bits.  */
    int condexec_mask;
    int condexec_cond;
    int thumb;
    /* Non-Secure mode, if TrustZone is available */
    bool ns;
    TTable *cp_regs;
    int user;
    int vfp_enabled;
    int vec_len;
    int vec_stride;
    /* M-profile ECI/ICI exception-continuable instruction state */
    int eci;
    /*
     * trans_ functions for insns which are continuable should set this true
     * after decode (ie after any UNDEF checks)
     */
    bool eci_handled;
    /* True if MVE insns are definitely not predicated by VPR or LTPSIZE */
    bool mve_no_pred;
} DisasContext;

/* ARM-specific interrupt pending bits.  */
#define CPU_INTERRUPT_FIQ CPU_INTERRUPT_TGT_EXT_1

#define NB_MMU_MODES 4

/* We currently assume float and double are IEEE single and double
   precision respectively.
   Doing runtime conversions is tricky because VFP registers may contain
   integer values (eg. as the result of a FTOSI instruction).
   s<2n> maps to the least significant half of d<n>
   s<2n+1> maps to the most significant half of d<n>
 */

#if defined(TARGET_ARM32)
#define CPU_PC(env) env->regs[15]
#elif defined(TARGET_ARM64)
#define CPU_PC(env) _CPU_PC(env)
inline uint64_t _CPU_PC(CPUState *env)
{
    //  TODO: check which mode we are in and optionally return env->regs[15]
    return env->pc;
}
#endif

//  +---------------------------------------+
//  | ALL FIELDS WHICH STATE MUST BE STORED |
//  | DURING SERIALIZATION SHOULD BE PLACED |
//  | BEFORE >CPU_COMMON< SECTION.          |
//  +---------------------------------------+
typedef struct CPUState {
    /* Regs for 32-bit current mode.  */
    uint32_t regs[16];
#ifdef TARGET_ARM64
    /* Regs for 64-bit mode. */
    uint64_t xregs[32];
    uint64_t pc;
#endif
    /* Frequently accessed CPSR bits are stored separately for efficiently.
       This contains all the other bits.  Use cpsr_{read,write} to access
       the whole CPSR.  */
    uint32_t uncached_cpsr;
    uint32_t spsr;

    /* Banked registers.  */
    uint32_t banked_spsr[6];
    uint32_t banked_r13[6];
    uint32_t banked_r14[6];

    /* These hold r8-r12.  */
    uint32_t usr_regs[5];
    uint32_t fiq_regs[5];

    /* cpsr flag cache for faster execution */
    uint32_t CF;            /* 0 or 1 */
    uint32_t VF;            /* V is the bit 31. All other bits are undefined */
    uint32_t NF;            /* N is bit 31. All other bits are undefined.  */
    uint32_t ZF;            /* Z set if zero.  */
    uint32_t QF;            /* 0 or 1 */
    uint32_t GE;            /* cpsr[19:16] */
    uint32_t thumb;         /* cpsr[5]. 0 = arm mode, 1 = thumb mode. */
    uint32_t condexec_bits; /* IT bits.  cpsr[15:10,26:25].  */
    bool secure;            /* Is CPU executing in Secure mode (TrustZone) */

    bool wfe;
    bool event_register;

    struct {
        uint32_t c1_dbgdrar; /* Debug ROM Address Register */
        uint32_t c2_dbgdsar; /* Debug Self Address Offset Register */
    } cp14;

    /* System control coprocessor (cp15) */
    struct {
        uint32_t c0_cpuid; /* Sometimes known as MIDR - Main ID Register */
        uint32_t c0_cachetype;
        uint32_t c0_tcmtype;     /* TCM type. */
        uint32_t c0_ccsid[16];   /* Cache size.  */
        uint32_t c0_clid;        /* Cache level.  */
        uint32_t c0_cssel;       /* Cache size selection.  */
        uint32_t c0_c1[8];       /* Feature registers.  */
        uint32_t c0_c2[8];       /* Instruction set registers.  */
        uint32_t c1_sys;         /* System control register.  */
        uint32_t c1_coproc;      /* Coprocessor access register.  */
        uint32_t c1_xscaleauxcr; /* XScale auxiliary control register.  */
        uint32_t c2_base0;       /* MMU translation table base 0.  */
        uint32_t c2_base1;       /* MMU translation table base 1.  */
        uint32_t c2_control;     /* MMU translation table base control.  */
        uint32_t c2_mask;        /* MMU translation table base selection mask.  */
        uint32_t c2_base_mask;   /* MMU translation table base 0 mask. */
        uint32_t c2_data;        /* MPU data cachable bits.  */
        uint32_t c2_insn;        /* MPU instruction cachable bits.  */
        uint32_t c2_ttbcr_eae;   /* Extended Address Enable: 0 - no LPAE, 1 - LPAE */
        uint64_t c2_base0_ea;    /* LPAE MMU translation table base 0 */
        uint32_t c3;             /* MMU domain access control register
                                    MPU write buffer control.  */
        uint32_t c5_insn;        /* Fault status registers.  */
        uint32_t c5_data;
        uint32_t c6_insn; /* Fault address registers.  */
        uint32_t c6_data;
        uint32_t c6_addr;
        uint32_t c6_base_address[MAX_MPU_REGIONS];      /* MPU base register.  */
        uint32_t c6_size_and_enable[MAX_MPU_REGIONS];   /* MPU size/enable register.  */
        uint32_t c6_access_control[MAX_MPU_REGIONS];    /* MPU access control register. */
        uint32_t c6_subregion_disable[MAX_MPU_REGIONS]; /* MPU subregion disable mask. This is not a hardware register */
        uint32_t c6_region_number;
        uint32_t c7_par;                           /* Translation result. */
        uint32_t c9_insn;                          /* Cache lockdown registers.  */
        uint32_t c9_tcmregion[2][MAX_TCM_REGIONS]; /* TCM Region Registers */
        uint32_t c9_tcmsel;                        /* TCM Selection Registers */
        uint32_t c9_data;
        uint32_t c9_pmcr;                /* performance monitor control register */
        uint32_t c9_pmcnten;             /* perf monitor counter enables */
        uint32_t c9_pmovsr;              /* perf monitor overflow status */
        uint32_t c9_pmxevtyper;          /* perf monitor event type */
        uint32_t c9_pmuserenr;           /* perf monitor user enable */
        uint32_t c9_pminten;             /* perf monitor interrupt enables */
        uint32_t c9_pmceid0;             /* perf monitor supported events */
        uint32_t c12_vbar;               /* vector base address register, security extensions*/
        uint32_t c13_fcse;               /* FCSE PID.  */
        uint32_t c13_context;            /* Context ID.  */
        uint32_t c13_tls1;               /* User RW Thread register.  */
        uint32_t c13_tls2;               /* User RO Thread register.  */
        uint32_t c13_tls3;               /* Privileged Thread register.  */
        uint32_t c15_cbar;               /* Configuration Base Address Register */
        uint32_t c15_cpar;               /* XScale Coprocessor Access Register */
        uint32_t c15_ticonfig;           /* TI925T configuration byte.  */
        uint32_t c15_i_max;              /* Maximum D-cache dirty line index.  */
        uint32_t c15_i_min;              /* Minimum D-cache dirty line index.  */
        uint32_t c15_threadid;           /* TI debugger thread-ID.  */
        uint32_t c15_decc_entries[3];    /* Cortex-R8: Data ECC entry no. 0-2 */
        uint32_t c15_iecc_entries[3];    /* Cortex-R8: Instruction ECC entry no. 0-2 */
        uint32_t c15_tcm_ecc_entries[2]; /* Cortex-R8: Data/Instruction TCM ECC entry */
        uint32_t c15_ahb_region;         /* Cortex-R5: AHB peripheral interface region register */
        uint32_t c15_axi_region;         /* Cortex-R5: LLPP Normal AXI peripheral interface region register */
        uint32_t c15_virtual_axi_region; /* Cortex-R5: LLPP Virtual AXI peripheral interface region register */
    } cp15;

    struct {
        bool counters_enabled;    /* Are PMU counters enabled? (E bit in PMCR) */
        uint16_t counters_number; /* Number of PM counters available for the current SoC */

        uint32_t cycles_divisor;
        uint32_t cycles_remainder; /* Cycles remainder if we use divisor */

        int selected_counter_id; /* Currently selected PMU counter */

        pmu_event implemented_events[PMU_EVENTS_NUMBER];     /* Supported PMU events */
        pmu_counter counters[PMU_MAX_PROGRAMMABLE_COUNTERS]; /* Individual PMU counter values */

        pmu_counter cycle_counter; /* Cycle counter - special case, since it has it's own fields all over the place, and is reset
                                      via Control Register */

        //  These are used to optimize calling PMU
        //  Touch them with extra caution
        int is_any_overflow_interrupt_enabled;  /* Have we enabled any overflow interrupt? */
        uint32_t cycles_overflow_nearest_limit; /* How many instructions left to cycles counting overflow */
        uint32_t insns_overflow_nearest_limit;  /* How many instructions left to instructions counting overflow */

        bool extra_logs_enabled;
    } pmu;

#ifdef TARGET_PROTO_ARM_M
    struct {
        /* These represent other (banked) stack pointers
         * Usually there are two (for handler and process),
         * but with TrustZone there are four (additional two for each mode)
         * They should be exchanged via `switch_v7m_sp`/`switch_v7m_security`.
         * regs[13] contains the current "active" pointer.
         * Use is_using_process_sp_current(env) to tell which stack pointer
         * should be used for the current state.
         * process_sp tells which stack pointer is currently in sp field.
         * It should always be equal to is_using_process_sp_current(env)
         * after finished state modification.
         */
        uint32_t other_sp;     /* Other Stack Pointer */
        uint32_t other_ss_msp; /* Other Security State Main Stack */
        uint32_t other_ss_psp; /* Other Security State Process Stack */
        bool process_sp;       /* Is the currently selected SP Process or Main SP */
        uint32_t vecbase[M_REG_NUM_BANKS];
        uint32_t basepri[M_REG_NUM_BANKS];
        /* SFPA and FPCA bits are not banked - required for FPU support in TrustZone.
         * When accessing them, make sure to always use `M_REG_COMMON` or `M_REG_S`
         * bank for this register. The rest of the bits are banked. */
        uint32_t control[M_REG_NUM_BANKS];
        uint32_t fault_status[M_REG_NUM_BANKS];
        /* BFSR and BFAR are not banked between Security states. See
         * rules IHGFM and ILRNV for their BFHFNMINS-dependent visibility. */
        uint32_t bus_fault_status;     /* BFSR */
        uint32_t bus_fault_address;    /* BFAR */
        uint32_t secure_fault_status;  /* SFSR */
        uint32_t secure_fault_address; /* SFAR. It can be shared with MMFAR, but it's more hassle, so let's keep it separate */
        uint32_t memory_fault_address[M_REG_NUM_BANKS];
        uint32_t exception;
        uint32_t primask[M_REG_NUM_BANKS];
        uint32_t faultmask[M_REG_NUM_BANKS];
        uint32_t ccr[M_REG_NUM_BANKS];
        uint32_t cpacr[M_REG_NUM_BANKS];
        uint32_t nsacr; /* RAZ/WI from Non-secure state */
        /* Generally, helpers: `fpccr_write/read` should be used to interact with it
         * but right now the Secure variant can as well be operated on directly
         * since it's a superset of Non-secure bits (including Non-banked ones
         * that live only in Secure mode) */
        uint32_t fpccr[M_REG_NUM_BANKS];
        uint32_t fpcar[M_REG_NUM_BANKS];
        uint32_t fpdscr[M_REG_NUM_BANKS];
        /* msplim/psplim are armv8-m specific */
        uint32_t msplim[M_REG_NUM_BANKS];
        uint32_t psplim[M_REG_NUM_BANKS];
        uint32_t has_trustzone;
        uint32_t ltpsize;
        uint32_t vpr;
        V7MExceptionPhase exception_phase;
        int32_t exception_phase_fault;
        /* A fault generated while returning from an exception reuses the
         * existing stack frame. The next exception entry consumes this saved
         * EXC_RETURN as a tail-chain instead of pushing another frame. */
        bool exception_return_tailchain;
        uint32_t exception_return_type;
        bool locked_up;
    } v7m;

    /* PMSAv8 MPUs */
    struct {
        uint32_t ctrl;
        uint32_t rnr;
        uint32_t rbar[MAX_MPU_REGIONS];
        uint32_t rlar[MAX_MPU_REGIONS];
        uint32_t mair[2]; /* The number of these registers is *not* configurable */
    } pmsav8[M_REG_NUM_BANKS];

    struct {
        uint32_t ctrl;
        uint32_t type;
        uint32_t rnr;
        uint32_t rbar[MAX_SAU_REGIONS];
        uint32_t rlar[MAX_SAU_REGIONS];
    } sau;

    uint32_t number_of_sau_regions;

    struct {
        bool enabled;
        bool custom_handler_enabled;
        uint32_t rbar[MAX_IDAU_REGIONS];
        uint32_t rlar[MAX_IDAU_REGIONS];
    } idau;

    uint32_t number_of_idau_regions;

    /* Additional memory attribution exemptions similar to the architecture-defined regions which
     * make security attribution of these regions (S/NS) the same as current CPU state.
     */
    struct {
        uint32_t count;
        uint32_t start[MAX_IMPL_DEF_ATTRIBUTION_EXEMPTIONS];
        uint32_t end[MAX_IMPL_DEF_ATTRIBUTION_EXEMPTIONS];
    } impl_def_attr_exemptions;

    int32_t sleep_on_exception_exit;
#endif

    /* Thumb-2 EE state.  */
    uint32_t teecr;
    uint32_t teehbr;

    /* Internal CPU feature flags.  */
    uint32_t features;

    /* VFP coprocessor state.  */
    struct {
        float64 regs[32];

        uint32_t xregs[16];
        /* We store these fpcsr fields separately for convenience.  */
        uint32_t qc;
        int vec_len;
        int vec_stride;

        /* scratch space when Tn are not sufficient.  */
        uint32_t scratch[8];

        /* fp_status is the "normal" fp status. standard_fp_status retains
         * values corresponding to the ARM "Standard FPSCR Value", ie
         * default-NaN, flush-to-zero, round-to-nearest and is used by
         * any operations (generally Neon) which the architecture defines
         * as controlled by the standard FPSCR value rather than the FPSCR.
         *
         * To avoid having to transfer exception bits around, we simply
         * say that the FPSCR cumulative exception flags are the logical
         * OR of the flags in the two fp statuses. This relies on the
         * only thing which needs to read the exception flags being
         * an explicit FPSCR read.
         */
        float_status fp_status;
        float_status standard_fp_status;
        /* fp_status equivalent for float16 instruction */
        float_status fp_status_f16;
        float_status standard_fp_status_f16;
#ifdef TARGET_PROTO_ARM_M
        int32_t fpu_interrupt_irq_number;
#endif
    } vfp;
    uint32_t exclusive_addr;
    uint32_t exclusive_val;
    uint32_t exclusive_high;

    int32_t sev_on_pending;

    /* iwMMXt coprocessor state.  */
    struct {
        uint64_t regs[16];
        uint64_t val;

        uint32_t cregs[16];
    } iwmmxt;

    uint32_t number_of_mpu_regions;

    CPU_COMMON

    /* Fields after CPU_COMMON are preserved on reset but not serialized as opposed to the ones before CPU_COMMON. */

    TTable *cp_regs;
} CPUState;

void switch_mode(CPUState *, int);

int cpu_handle_mmu_fault(CPUState *env, target_ulong address, int rw, int mmu_idx, int no_page_fault, target_phys_addr_t *paddr);

#define PRIMASK_EN 1

#define CPSR_M        (0x1f)
#define CPSR_T        (1 << 5)
#define CPSR_F        (1 << 6)
#define CPSR_I        (1 << 7)
#define CPSR_A        (1 << 8)
#define CPSR_E        (1 << 9)
#define CPSR_IT_2_7   (0xfc00)
#define CPSR_GE       (0xf << 16)
#define CPSR_RESERVED (0xf << 20)
#define CPSR_J        (1 << 24)
#define CPSR_IT_0_1   (3 << 25)
#define CPSR_Q        (1 << 27)
#define CPSR_V        (1 << 28)
#define CPSR_C        (1 << 29)
#define CPSR_Z        (1 << 30)
#define CPSR_N        (1 << 31)
#define CPSR_NZCV     (CPSR_N | CPSR_Z | CPSR_C | CPSR_V)

#define CPSR_IT          (CPSR_IT_0_1 | CPSR_IT_2_7)
#define CACHED_CPSR_BITS (CPSR_T | CPSR_GE | CPSR_IT | CPSR_Q | CPSR_NZCV)
/* Bits writable in user mode.  */
#define CPSR_USER (CPSR_NZCV | CPSR_Q | CPSR_GE)
/* Execution state bits.  MRS read as zero, MSR writes ignored.  */
#define CPSR_EXEC (CPSR_T | CPSR_IT | CPSR_J)

/* Return the current CPSR value.  */
uint32_t cpsr_read(CPUState *env);
/* Set the CPSR.  Note that some bits of mask must be all-set or all-clear.  */
void cpsr_write(CPUState *env, uint32_t val, uint32_t mask);

#ifdef TARGET_PROTO_ARM_M
/* Return the current xPSR value.  */
static inline uint32_t xpsr_read(CPUState *env)
{
    int ZF;
    ZF = (env->ZF == 0);
    return (env->NF & 0x80000000) | (ZF << 30) | (env->CF << 29) | ((env->VF & 0x80000000) >> 3) | (env->QF << 27) |
           (env->thumb << 24) | ((env->condexec_bits & 3) << 25) | ((env->condexec_bits & 0xfc) << 8) | env->v7m.exception;
}

/* Set the xPSR.  Note that some bits of mask must be all-set or all-clear.  */
static inline void xpsr_write(CPUState *env, uint32_t val, uint32_t mask)
{
    if(mask & CPSR_NZCV) {
        env->ZF = (~val) & CPSR_Z;
        env->NF = val;
        env->CF = (val >> 29) & 1;
        env->VF = (val << 3) & 0x80000000;
    }
    if(mask & CPSR_Q) {
        env->QF = ((val & CPSR_Q) != 0);
    }
    if(mask & (1 << 24)) {
        env->thumb = ((val & (1 << 24)) != 0);
    }
    if(mask & CPSR_IT_0_1) {
        env->condexec_bits &= ~3;
        env->condexec_bits |= (val >> 25) & 3;
    }
    if(mask & CPSR_IT_2_7) {
        env->condexec_bits &= 3;
        env->condexec_bits |= (val >> 8) & 0xfc;
    }
    if(mask & 0x1ff) {
        env->v7m.exception = val & 0x1ff;
    }
}

void vfp_trigger_exception();

bool need_fp_lazy_state_preservation(CPUState *env);
bool need_fp_context(CPUState *env);

//  More secure options intentionally have greater numbers so take care when modifying this enum.
//  Make sure any security attribution comparisons, e.g. in the functions below, are still correct.
enum security_attribution {
    SA_NONSECURE,
    SA_SECURE_NSC,
    SA_SECURE,
};

static inline enum security_attribution attribution_get_more_secure(enum security_attribution attribution1,
                                                                    enum security_attribution attribution2)
{
    //  The enum is "in ascending order of security" so we can just choose the greatest value
    //  according to the ARMv8-M manual rule JGHS.
    return attribution1 >= attribution2 ? attribution1 : attribution2;
}

static inline bool attribution_is_secure(enum security_attribution attrib)
{
    return attrib >= SA_SECURE_NSC;
}

#define SAU_CTRL_ENABLE 0x01
#define SAU_CTRL_ALLNS  0x02

/* There are no such registers in cores, we simply imitate SAU. */
#define IDAU_SAU_RLAR_ENABLE 0x01
#define IDAU_SAU_RLAR_NSC    0x02
#endif

/* Return the current FPSCR value.  */
uint32_t vfp_get_fpscr(CPUState *env);
void vfp_set_fpscr(CPUState *env, uint32_t val);

enum arm_fpdecoderm {
    FPROUNDING_TIEAWAY,
    FPROUNDING_TIEEVEN,
    FPROUNDING_POSINF,
    FPROUNDING_NEGINF,
    FPROUNDING_ZERO,
};

enum arm_cpu_mode {
    ARM_CPU_MODE_USR = 0x10,
    ARM_CPU_MODE_FIQ = 0x11,
    ARM_CPU_MODE_IRQ = 0x12,
    ARM_CPU_MODE_SVC = 0x13,
    ARM_CPU_MODE_ABT = 0x17,
    ARM_CPU_MODE_UND = 0x1b,
    ARM_CPU_MODE_SYS = 0x1f,
    /* Legacy 26-bit modes.
       They are treated as aliases for the corresponding 32-bit mode. */
    ARM_CPU_MODE_USR26 = 0x00,
    ARM_CPU_MODE_FIQ26 = 0x01,
    ARM_CPU_MODE_IRQ26 = 0x02,
    ARM_CPU_MODE_SVC26 = 0x03,
};

enum arm_fp_precision {
    HALF_PRECISION = 1,
    SINGLE_PRECISION = 2,
    DOUBLE_PRECISION = 3,
};

#ifdef TARGET_PROTO_ARM_M
static inline bool in_handler_mode(CPUState *env)
{
    return env->v7m.exception != 0;
}

static inline bool is_using_process_sp(CPUState *env, bool is_handler, bool is_secure)
{
    return !is_handler && (env->v7m.control[is_secure] & 2);
}

static inline bool is_using_process_sp_current(CPUState *env)
{
    return is_using_process_sp(env, in_handler_mode(env), env->secure);
}

static inline uint32_t *pointer_msp(CPUState *env)
{
    if(env->v7m.process_sp) {
        return &env->v7m.other_sp;
    } else {
        return &env->regs[13];
    }
}

static inline uint32_t *pointer_psp(CPUState *env)
{
    if(env->v7m.process_sp) {
        return &env->regs[13];
    } else {
        return &env->v7m.other_sp;
    }
}

static inline void set_msp(CPUState *env, uint32_t value)
{
    *pointer_msp(env) = value;
}

static inline uint32_t get_msp(CPUState *env)
{
    return *pointer_msp(env);
}

static inline void set_psp(CPUState *env, uint32_t value)
{
    *pointer_psp(env) = value;
}

static inline uint32_t get_psp(CPUState *env)
{
    return *pointer_psp(env);
}

static inline bool in_privileged_mode_with_security(CPUState *env, bool is_secure)
{
    return in_handler_mode(env) || !(env->v7m.control[is_secure] & 1);
}

static inline bool in_privileged_mode(CPUState *env)
{
    return in_privileged_mode_with_security(env, env->secure);
}

static inline bool is_requested_exception_priority_negative(CPUState *env, bool secure)
{
    /* ARM: IsReqExcPriNeg */
    return env->v7m.exception == ARMV7M_EXCP_NMI || env->v7m.exception == ARMV7M_EXCP_HARD || env->v7m.faultmask[secure];
}
#endif

static inline bool in_user_mode(CPUState *env)
{
#ifdef TARGET_PROTO_ARM_M
    return !in_privileged_mode(env);
#else
    return (env->uncached_cpsr & CPSR_M) == ARM_CPU_MODE_USR;
#endif
}

/* VFP system registers.  */
#define ARM_VFP_FPSID   0
#define ARM_VFP_FPSCR   1
#define ARM_VFP_MVFR1   6
#define ARM_VFP_MVFR0   7
#define ARM_VFP_FPEXC   8
#define ARM_VFP_FPINST  9
#define ARM_VFP_FPINST2 10

/* These ones are M-profile only */
#define ARM_VFP_FPSCR_NZCVQC 2
#define ARM_VFP_VPR          12
#define ARM_VFP_P0           13
#define ARM_VFP_FPCXT_NS     14
#define ARM_VFP_FPCXT_S      15

/* Internal value meaning "FPSCR, but we care only about NZCV" */
#define INTERNAL_VFP_FPSCR_NZCV 0xffff

/* FP fields.  */
#define ARM_CONTROL_FPCA    2
#define ARM_CONTROL_SFPA    3
#define ARM_FPCCR_LSPACT    0
#define ARM_FPCCR_S         2
#define ARM_FPCCR_TS        26
#define ARM_FPCCR_CLRONRETS 27
#define ARM_FPCCR_CLRONRET  28
#define ARM_FPCCR_LSPENS    29
#define ARM_FPCCR_LSPEN     30
#define ARM_FPCCR_ASPEN     31
#define ARM_FPCAR_ADDRESS   3
#define ARM_VFP_FPEXC_FPUEN 30

#define ARM_EXC_RETURN_ES    0
#define ARM_EXC_RETURN_SPSEL 2
#define ARM_EXC_RETURN_MODE  3
#define ARM_EXC_RETURN_NFPCA 4 /* Also known as EXC_RETURN.FType */
#define ARM_EXC_RETURN_DCRS  5
#define ARM_EXC_RETURN_S     6

#define ARM_CONTROL_FPCA_MASK    (1 << ARM_CONTROL_FPCA)
#define ARM_CONTROL_SFPA_MASK    (1 << ARM_CONTROL_SFPA)
#define ARM_FPCCR_LSPACT_MASK    (1 << ARM_FPCCR_LSPACT)
#define ARM_FPCCR_S_MASK         (1 << ARM_FPCCR_S)
#define ARM_FPCCR_HFRDY_MASK     (1 << 4)
#define ARM_FPCCR_BFRDY_MASK     (1 << 6)
#define ARM_FPCCR_SFRDY_MASK     (1 << 7)
#define ARM_FPCCR_MONRDY_MASK    (1 << 8)
#define ARM_FPCCR_TS_MASK        (1 << ARM_FPCCR_TS)
#define ARM_FPCCR_CLRONRETS_MASK (1 << ARM_FPCCR_CLRONRETS)
#define ARM_FPCCR_CLRONRET_MASK  (1 << ARM_FPCCR_CLRONRET)
#define ARM_FPCCR_LSPENS_MASK    (1 << ARM_FPCCR_LSPENS)
#define ARM_FPCCR_LSPEN_MASK     (1 << ARM_FPCCR_LSPEN)
#define ARM_FPCCR_ASPEN_MASK     (1 << ARM_FPCCR_ASPEN)
#define ARM_FPCAR_ADDRESS_MASK   (0xfffffff8)
#define ARM_VFP_FPEXC_FPUEN_MASK (1 << ARM_VFP_FPEXC_FPUEN)
#define ARM_FPDSCR_VALUES_MASK   0x07c00000

#define ARM_FPCCR_COMMON_READY_MASK (ARM_FPCCR_HFRDY_MASK | ARM_FPCCR_BFRDY_MASK | ARM_FPCCR_SFRDY_MASK | ARM_FPCCR_MONRDY_MASK)

#define ARM_EXC_RETURN_ES_MASK    (1 << ARM_EXC_RETURN_ES)
#define ARM_EXC_RETURN_SPSEL_MASK (1 << ARM_EXC_RETURN_SPSEL)
#define ARM_EXC_RETURN_MODE_MASK  (1 << ARM_EXC_RETURN_MODE)
#define ARM_EXC_RETURN_NFPCA_MASK (1 << ARM_EXC_RETURN_NFPCA)
#define ARM_EXC_RETURN_DCRS_MASK  (1 << ARM_EXC_RETURN_DCRS)
#define ARM_EXC_RETURN_S_MASK     (1 << ARM_EXC_RETURN_S)

#define RETPSR_SFPA (1 << 20)

#define ARM_CPACR_CP10      20
#define ARM_CPACR_CP10_MASK (3 << ARM_CPACR_CP10)

#define ARM_NSACR_CP10      10
#define ARM_NSACR_CP10_MASK (1 << ARM_NSACR_CP10)

#define ARM_CPN_ACCESS_NONE 0
#define ARM_CPN_ACCESS_PRIV 1
#define ARM_CPN_ACCESS_FULL 3

/* iwMMXt coprocessor control registers.  */
#define ARM_IWMMXT_wCID  0
#define ARM_IWMMXT_wCon  1
#define ARM_IWMMXT_wCSSF 2
#define ARM_IWMMXT_wCASF 3
#define ARM_IWMMXT_wCGR0 8
#define ARM_IWMMXT_wCGR1 9
#define ARM_IWMMXT_wCGR2 10
#define ARM_IWMMXT_wCGR3 11

/*  This enum is used in Renode-infrastructure/src/Emulator/Cores/Arm/Arm.cs
    Every change is this enum MUST be replicated in that file.
*/
enum arm_features {
    ARM_FEATURE_VFP,
    ARM_FEATURE_AUXCR,  /* ARM1026 Auxiliary control register.  */
    ARM_FEATURE_XSCALE, /* Intel XScale extensions.  */
    ARM_FEATURE_IWMMXT, /* Intel iwMMXt extension.  */
    ARM_FEATURE_V6,
    ARM_FEATURE_V6K,
    ARM_FEATURE_V7,
    ARM_FEATURE_V7SEC, /* v7 Security Extensions */
    ARM_FEATURE_THUMB2,
    ARM_FEATURE_MPU, /* Only has Memory Protection Unit, not full MMU.  */
    ARM_FEATURE_VFP3,
    ARM_FEATURE_VFP_FP16,
    ARM_FEATURE_NEON,
    ARM_FEATURE_THUMB_DIV, /* divide supported in Thumb encoding */
    ARM_FEATURE_OMAPCP,    /* OMAP specific CP15 ops handling.  */
    ARM_FEATURE_THUMB2EE,
    ARM_FEATURE_V7MP, /* v7 Multiprocessing Extensions */
    ARM_FEATURE_V4T,
    ARM_FEATURE_V5,
    ARM_FEATURE_STRONGARM,
    ARM_FEATURE_VAPA,    /* cp15 VA to PA lookups */
    ARM_FEATURE_ARM_DIV, /* divide supported in ARM encoding */
    ARM_FEATURE_VFP4,    /* VFPv4 (implies that NEON is v2) */
    ARM_FEATURE_VFP5,
    ARM_FEATURE_GENERIC_TIMER,
    ARM_FEATURE_V8, /* implies PMSAv8 MPU and VFP5 if a FPU is present*/
    ARM_FEATURE_PMSA,
    ARM_FEATURE_DSP,
    ARM_FEATURE_MVE,
    ARM_FEATURE_CBAR_RO, /* has cp15 CBAR and it is read-only */
    ARM_FEATURE_LPAE,    /* v7 Only, Large Physical Address Extension */
    ARM_FEATURE_V8_1M,
};

/* Values for M-profile PSR.ECI for MVE insns */
enum MVEECIState {
    ECI_NONE = 0, /* No completed beats */
    ECI_A0 = 1,   /* Completed: A0 */
    ECI_A0A1 = 2, /* Completed: A0, A1 */
    /* 3 is reserved */
    ECI_A0A1A2 = 4,   /* Completed: A0, A1, A2 */
    ECI_A0A1A2B0 = 5, /* Completed: A0, A1, A2, B0 */
    /* All other values reserved */
};

static inline int arm_feature(const CPUState *env, int feature)
{
    return (env->features & (1u << feature)) != 0;
}

/* Does the core conform to the the "MicroController" profile. e.g. Cortex-M3.
   Note the M in older cores (eg. ARM7TDMI) stands for Multiply. These are
   conventional cores (ie. Application or Realtime profile).  */

#define ARM_CPUID(env) (env->cp15.c0_cpuid)

//  MIDR, Main ID Register value
#define ARM_CPUID_ARM7TDMI       0x40700f0f
#define ARM_CPUID_ARM1026EJ_S    0x4106a262
#define ARM_CPUID_ARM926EJ_S     0x41069265
#define ARM_CPUID_ARM946E_S      0x41059461
#define ARM_CPUID_ARM1136JF_S    0x4117b363
#define ARM_CPUID_ARM1136JF_S_R2 0x4107b362
#define ARM_CPUID_ARM1176JZF_S   0x410fb767
#define ARM_CPUID_TI915T         0x54029152
#define ARM_CPUID_TI925T         0x54029252
#define ARM_CPUID_SA1100         0x4401A11B
#define ARM_CPUID_SA1110         0x6901B119
#define ARM_CPUID_PXA250         0x69052100
#define ARM_CPUID_PXA255         0x69052d00
#define ARM_CPUID_PXA260         0x69052903
#define ARM_CPUID_PXA261         0x69052d05
#define ARM_CPUID_PXA262         0x69052d06
#define ARM_CPUID_PXA270         0x69054110
#define ARM_CPUID_PXA270_A0      0x69054110
#define ARM_CPUID_PXA270_A1      0x69054111
#define ARM_CPUID_PXA270_B0      0x69054112
#define ARM_CPUID_PXA270_B1      0x69054113
#define ARM_CPUID_PXA270_C0      0x69054114
#define ARM_CPUID_PXA270_C5      0x69054117
#define ARM_CPUID_ARM11MPCORE    0x410fb022
#define ARM_CPUID_CORTEXA5       0x410fc050
#define ARM_CPUID_CORTEXA7       0x410fc070
#define ARM_CPUID_CORTEXA8       0x410fc080
#define ARM_CPUID_CORTEXA9       0x410fc090
#define ARM_CPUID_CORTEXA15      0x412fc0f1
#define ARM_CPUID_CORTEXM0       0x410cc200
#define ARM_CPUID_CORTEXM23      0x411cd200
#define ARM_CPUID_CORTEXM3       0x410fc231
#define ARM_CPUID_CORTEXM33      0x411fd210
#define ARM_CPUID_CORTEXM4       0x410fc240
#define ARM_CPUID_CORTEXM52      0x630fd243
#define ARM_CPUID_CORTEXM55      0x411fd221
#define ARM_CPUID_CORTEXM7       0x411fc272
#define ARM_CPUID_CORTEXM85      0x411fd230
#define ARM_CPUID_CORTEXR5       0x410fc150
#define ARM_CPUID_CORTEXR5F      0x410fc151
#define ARM_CPUID_CORTEXR8       0x410fc183
#define ARM_CPUID_ANY            0xffffffff

/* The ARM MMU allows 1k pages.  */
/* ??? Linux doesn't actually use these, and they're deprecated in recent
   architecture revisions.  Maybe a configure option to disable them.  */
#define TARGET_PAGE_BITS 10

#define TARGET_PHYS_ADDR_SPACE_BITS 32

/* MMU modes definitions */

typedef union {
    struct {
        bool user : 1;
        bool secure : 1;
        /* This field ensures that index is always initialized.
         * Size of individual bits plus this field has to be equal to the size of the index */
        int : 30;
    };
    int32_t index;
} MMUMode;

static inline MMUMode context_to_mmu_mode(DisasContext *s)
{
    MMUMode mode = {
        .user = s->user,
        .secure = !s->ns,
    };
    return mode;
}

static inline int context_to_mmu_index(DisasContext *s)
{
    return context_to_mmu_mode(s).index;
}

//  Don't rename, it's used in softmmu.
static inline int cpu_mmu_index(CPUState *env)
{
    MMUMode mode = {
        .user = in_user_mode(env),
        .secure = env->secure,
    };
    return mode.index;
}

static inline MMUMode mmu_index_to_mode(int index)
{
    return (MMUMode) { .index = index };
}

#include "cpu-all.h"

enum mpu_result {
    MPU_SUCCESS = TRANSLATE_SUCCESS,
    MPU_PERMISSION_FAULT = TRANSLATE_FAIL,
    MPU_BACKGROUND_FAULT,
};

/* Bit usage in the TB flags field: */
#define ARM_TBFLAG_THUMB_SHIFT 0
#define ARM_TBFLAG_THUMB_MASK  (1 << ARM_TBFLAG_THUMB_SHIFT)

//  NOTE: VECLEN (Cortex-A) and LTPSIZE (Cortex-M) occupy the same place in TBFLAG
#define ARM_TBFLAG_VECLEN_SHIFT  1
#define ARM_TBFLAG_VECLEN_MASK   (0x7 << ARM_TBFLAG_VECLEN_SHIFT)
#define ARM_TBFLAG_LTPSIZE_SHIFT 1
#define ARM_TBFLAG_LTPSIZE_MASK  (0x7 << ARM_TBFLAG_LTPSIZE_SHIFT)

#define ARM_TBFLAG_VECSTRIDE_SHIFT 4
#define ARM_TBFLAG_VECSTRIDE_MASK  (0x3 << ARM_TBFLAG_VECSTRIDE_SHIFT)
#define ARM_TBFLAG_PRIV_SHIFT      6
#define ARM_TBFLAG_PRIV_MASK       (1 << ARM_TBFLAG_PRIV_SHIFT)
#define ARM_TBFLAG_VFPEN_SHIFT     7
#define ARM_TBFLAG_VFPEN_MASK      (1 << ARM_TBFLAG_VFPEN_SHIFT)
#define ARM_TBFLAG_CONDEXEC_SHIFT  8
#define ARM_TBFLAG_CONDEXEC_MASK   (0xff << ARM_TBFLAG_CONDEXEC_SHIFT)
#define ARM_TBFLAG_NS_SHIFT        16
#define ARM_TBFLAG_NS_MASK         (1 << ARM_TBFLAG_NS_SHIFT)
/* Bits 31..17 are currently unused. */

/* some convenience accessor macros */
#define ARM_TBFLAG_THUMB(F)     (((F) & ARM_TBFLAG_THUMB_MASK) >> ARM_TBFLAG_THUMB_SHIFT)
#define ARM_TBFLAG_VECLEN(F)    (((F) & ARM_TBFLAG_VECLEN_MASK) >> ARM_TBFLAG_VECLEN_SHIFT)
#define ARM_TBFLAG_VECSTRIDE(F) (((F) & ARM_TBFLAG_VECSTRIDE_MASK) >> ARM_TBFLAG_VECSTRIDE_SHIFT)
#define ARM_TBFLAG_PRIV(F)      (((F) & ARM_TBFLAG_PRIV_MASK) >> ARM_TBFLAG_PRIV_SHIFT)
#define ARM_TBFLAG_VFPEN(F)     (((F) & ARM_TBFLAG_VFPEN_MASK) >> ARM_TBFLAG_VFPEN_SHIFT)
#define ARM_TBFLAG_CONDEXEC(F)  (((F) & ARM_TBFLAG_CONDEXEC_MASK) >> ARM_TBFLAG_CONDEXEC_SHIFT)
#define ARM_TBFLAG_NS(F)        (((F) & ARM_TBFLAG_NS_MASK) >> ARM_TBFLAG_NS_SHIFT)

static inline void cpu_get_tb_cpu_state(CPUState *env, target_ulong *pc, target_ulong *cs_base, int *flags)
{
    int privmode;
    *pc = CPU_PC(env);
    *cs_base = 0;
    *flags = (env->thumb << ARM_TBFLAG_THUMB_SHIFT) | (env->vfp.vec_stride << ARM_TBFLAG_VECSTRIDE_SHIFT) |
             (env->condexec_bits << ARM_TBFLAG_CONDEXEC_SHIFT) | (!env->secure << ARM_TBFLAG_NS_SHIFT);

#ifndef TARGET_PROTO_ARM_M
    *flags |= (env->vfp.vec_len << ARM_TBFLAG_VECLEN_SHIFT);
#else
    *flags |= (env->v7m.ltpsize << ARM_TBFLAG_LTPSIZE_SHIFT);
#endif

    privmode = !in_user_mode(env);
    if(privmode) {
        *flags |= ARM_TBFLAG_PRIV_MASK;
    }

    if((env->vfp.xregs[ARM_VFP_FPEXC] & ARM_VFP_FPEXC_FPUEN_MASK)
#ifdef TARGET_PROTO_ARM_M
       /* We encode "env->secure" before into flags */
       /* CPU either has to be in privileged mode or have CPACR CP10 field set to allow access from non privileged mode. */
       && (privmode || ((env->v7m.cpacr[env->secure] & ARM_CPACR_CP10_MASK) >> ARM_CPACR_CP10) == ARM_CPN_ACCESS_FULL)
       /* CPU either has to be in Secure state or have NSACR CP10 field set to allow access from Non-secure state */
       && (env->secure || (env->v7m.nsacr & ARM_NSACR_CP10_MASK))
#endif
    ) {
        *flags |= ARM_TBFLAG_VFPEN_MASK;
    }
}

static inline bool is_cpu_event_pending(CPUState *env)
{
    //  The execution of an SEV instruction on any processor in the multiprocessor system.
    bool event_pending = env->event_register;
#ifdef TARGET_PROTO_ARM_M
    //  Any exception entering the Pending state if SEVONPEND in the System Control Register is set.
    event_pending |= env->sev_on_pending && tlib_nvic_get_pending_masked_irq();
    //  An asynchronous exception at a priority that preempts any currently active exceptions.
    event_pending |= is_interrupt_pending(env, CPU_INTERRUPT_HARD);
#else
    uint32_t cpsr = cpsr_read(env);
    //  An IRQ interrupt (even when CPSR I-bit is set, some implementations check this mask)
    event_pending |= is_interrupt_pending(env, CPU_INTERRUPT_HARD);
    //  An FIQ interrupt (even when CPSR F-bit is set, some implementations check this mask)
    event_pending |= is_interrupt_pending(env, CPU_INTERRUPT_FIQ);
    //  An asynchronous abort (not when masked by the CPSR A-bit)
    event_pending |= is_interrupt_pending(env, CPU_INTERRUPT_EXITTB) && (cpsr & CPSR_A);
    //  Events could be sent by implementation defined mechanisms, e.g.:
    //  A CP15 maintenance request broadcast by other processors.
    //  Virtual Interrupts (HCR), Hypervisor mode isn't implemented
    //  TODO
#endif

    return event_pending;
}

static inline bool cpu_has_work(CPUState *env)
{
    if(env->wfe && is_cpu_event_pending(env)) {
        env->wfe = 0;
    }

    if(env->wfi) {
        bool has_work = false;
#ifndef TARGET_PROTO_ARM_M
        has_work = is_interrupt_pending(env, CPU_INTERRUPT_FIQ | CPU_INTERRUPT_HARD | CPU_INTERRUPT_EXITTB);
#else
        has_work = tlib_nvic_get_pending_masked_irq() != 0;
#endif
        if(has_work) {
            env->wfi = 0;
        }
    }

    return !(env->wfe || env->wfi);
}

#include "exec-all.h"

static inline void cpu_pc_from_tb(CPUState *env, TranslationBlock *tb)
{
#if defined(TARGET_ARM32)
    env->regs[15] = tb->pc;
#elif defined(TARGET_ARM64)
    //  TODO: check mode
    env->pc = tb->pc;
#endif
}

void do_v7m_exception_exit(CPUState *env);
void do_v7m_secure_return(CPUState *env);
void v7m_set_locked_up(CPUState *env, bool locked_up);
void v7m_enter_reset_lockup(CPUState *env);

#ifdef TARGET_PROTO_ARM_M
static inline bool automatic_sleep_after_interrupt(CPUState *env)
{
    if(env->sleep_on_exception_exit) {
        env->wfi = 1;
        return true;
    }
    return false;
}
#endif

static inline void refresh_pending_irq(void)
{
#ifdef TARGET_PROTO_ARM_M
    /* The NVIC owns the Armv8-M execution-priority calculation, including
     * both banks of PRIMASK, BASEPRI, and FAULTMASK. Always let it refresh
     * the external IRQ line after architectural state changes. */
    tlib_nvic_find_pending_irq();
#endif
}

static inline enum arm_cpu_mode cpu_get_current_execution_mode()
{
    return env->uncached_cpsr & CPSR_M;
}

#ifdef TARGET_PROTO_ARM_M
void switch_v7m_sp(CPUState *env, bool process);

uint32_t ccr_read(CPUState *env, bool is_secure);
void ccr_write(CPUState *env, uint32_t value, bool is_secure);

static inline uint32_t fpccr_read(CPUState *env, bool is_secure)
{
    /* Some bits are RES0 [25:11] inclusive */
    uint32_t read_mask = ~0x3FFF800;
    if(!is_secure) {
        /* Bits marked as RAZ if read from Non-secure */
        read_mask ^= ARM_FPCCR_TS_MASK | ARM_FPCCR_S_MASK | ARM_FPCCR_LSPENS_MASK | ARM_FPCCR_CLRONRETS_MASK;
    }
    uint32_t fpccr = env->v7m.fpccr[is_secure] & read_mask;
    if(!env->v7m.has_trustzone) {
        /* With no Security Extension there is only one architectural FPCCR.
         * The implementation still uses the NS array slot for banked fields
         * and the S slot for fields which would be common. */
        fpccr |= env->v7m.fpccr[M_REG_COMMON] & (ARM_FPCCR_HFRDY_MASK | ARM_FPCCR_BFRDY_MASK | ARM_FPCCR_MONRDY_MASK);
    }
    /* LSPEN is not banked, and always readable */
    fpccr |= env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_LSPEN_MASK;
    /* Same for CLRONRET */
    fpccr |= env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_CLRONRET_MASK;
    return fpccr;
}

static inline void fpccr_write(CPUState *env, uint32_t value, bool is_secure)
{
    /* Start with all enabled, since there are fields we don't support
     * but the software might expect them to be writable */
    uint32_t write_mask = UINT32_MAX;
    if(!is_secure) {
        /* These are not banked, but exist only in Secure mode */
        write_mask ^= ARM_FPCCR_TS_MASK | ARM_FPCCR_S_MASK | ARM_FPCCR_LSPENS_MASK | ARM_FPCCR_LSPEN_MASK |
                      ARM_FPCCR_CLRONRETS_MASK | ARM_FPCCR_CLRONRET_MASK;
        /* LSPEN is only writable if LSPENS is unset */
        if((env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_LSPENS_MASK) == 0) {
            env->v7m.fpccr[M_REG_COMMON] =
                deposit32(env->v7m.fpccr[M_REG_COMMON], ARM_FPCCR_LSPEN, 1, value & ARM_FPCCR_LSPEN_MASK ? 1 : 0);
        }
        /* Similarly for CLRONRETS */
        if((env->v7m.fpccr[M_REG_COMMON] & ARM_FPCCR_CLRONRETS_MASK) == 0) {
            env->v7m.fpccr[M_REG_COMMON] =
                deposit32(env->v7m.fpccr[M_REG_COMMON], ARM_FPCCR_CLRONRET, 1, value & ARM_FPCCR_CLRONRET_MASK ? 1 : 0);
        }
    }
    env->v7m.fpccr[is_secure] = value & write_mask;
}

//  Region granularity is 32B, the remaining RBAR/RLAR bits contain flags like region enabled etc.
#define PMSAV8_IDAU_SAU_REGION_GRANULARITY_B 0x20u
#define PMSAV8_IDAU_SAU_FLAGS_MASK           (PMSAV8_IDAU_SAU_REGION_GRANULARITY_B - 1u)
#define PMSAV8_IDAU_SAU_ADDRESS_MASK         (~PMSAV8_IDAU_SAU_FLAGS_MASK)

static inline uint32_t pmsav8_idau_sau_get_region_base(uint32_t address_or_rbar)
{
    return address_or_rbar & PMSAV8_IDAU_SAU_ADDRESS_MASK;
}

static inline uint32_t pmsav8_idau_sau_get_region_limit(uint32_t address_or_rlar)
{
    return address_or_rlar | PMSAV8_IDAU_SAU_FLAGS_MASK;
}

static inline uint32_t pmsav8_idau_sau_get_flags(uint32_t rbar_or_rlar)
{
    return rbar_or_rlar & PMSAV8_IDAU_SAU_FLAGS_MASK;
}

#undef PMSAV8_IDAU_SAU_FLAGS_MASK
#undef PMSAV8_IDAU_SAU_ADDRESS_MASK

#endif  //  TARGET_PROTO_ARM_M

//  FIELD expands to constants:
//  * __REGISTER_<register>_<field>_START,
//  * __REGISTER_<register>_<field>_WIDTH.
//  * __REGISTER_<register>_<field>_MASK.
#define FIELD(register, field, start_bit, width)                               \
    static const unsigned __REGISTER_##register##_##field##_START = start_bit; \
    static const unsigned __REGISTER_##register##_##field##_WIDTH = width;     \
    static const unsigned __REGISTER_##register##_##field##_MASK = MAKE_64BIT_MASK(start_bit, width);

#define FIELD_DP32(variable, register, field, value) \
    deposit32(variable, __REGISTER_##register##_##field##_START, __REGISTER_##register##_##field##_WIDTH, value)

#define FIELD_EX32(variable, register, field) \
    extract32(variable, __REGISTER_##register##_##field##_START, __REGISTER_##register##_##field##_WIDTH)

#define TCG_GEN_FIELD_DP32(result, destination, source, register, field)                      \
    tcg_gen_deposit_i32(result, destination, source, __REGISTER_##register##_##field##_START, \
                        __REGISTER_##register##_##field##_WIDTH)

#define TCG_GEN_FIELD_EX32(result, source, register, field) \
    tcg_gen_extract_i32(result, source, __REGISTER_##register##_##field##_START, __REGISTER_##register##_##field##_WIDTH)

#define FIELD_MASK(register, field) __REGISTER_##register##_##field##_MASK

#define DEBUG_ADDRESS_VALID_VALUE 0b11

FIELD(DBGDRAR, ROMADDR, 12, 20)
FIELD(DBGDRAR, Valid, 0, 2)

FIELD(DBGDSAR, SELFOFFSET, 12, 20)
FIELD(DBGDSAR, Valid, 0, 2)

FIELD(ITCMRR, BASE_ADDRESS, 12, 20)
FIELD(ITCMRR, SIZE, 2, 5)
FIELD(ITCMRR, ENABLE_BIT, 0, 1)

FIELD(SCTLR, V, 13, 1)

FIELD(V7M_VPR, P0, 0, 16)
FIELD(V7M_VPR, MASK01, 16, 4)
FIELD(V7M_VPR, MASK23, 20, 4)

/* FPSCR register */
FIELD(VFP_FPSCR, IOC, 0, 1)
FIELD(VFP_FPSCR, DZC, 1, 1)
FIELD(VFP_FPSCR, OFC, 2, 1)
FIELD(VFP_FPSCR, UFC, 3, 1)
FIELD(VFP_FPSCR, IXC, 4, 1)
/* Bits [6:5] are RES0  on Cortex-M */
FIELD(VFP_FPSCR, IDC, 7, 1)
/* Bits [15:8] are RES0 on Cortex-M */

#ifdef TARGET_PROTO_ARM_M
FIELD(VFP_FPSCR, LTPSIZE, 16, 3)
#else
FIELD(VFP_FPSCR, VEC_LEN, 16, 3)
#endif

FIELD(VFP_FPSCR, FZ16, 19, 1)

#ifndef TARGET_PROTO_ARM_M
FIELD(VFP_FPSCR, VEC_STRIDE, 20, 2)
#endif /* Bits [21:20] are RES0 on Cortex-M */

FIELD(VFP_FPSCR, RMode, 22, 2)
FIELD(VFP_FPSCR, FZ, 24, 1)
FIELD(VFP_FPSCR, DN, 25, 1)
FIELD(VFP_FPSCR, AHP, 26, 1)
FIELD(VFP_FPSCR, QC, 27, 1) /* RES0 if MVE is not implemented */
FIELD(VFP_FPSCR, V, 28, 1)
FIELD(VFP_FPSCR, C, 29, 1)
FIELD(VFP_FPSCR, Z, 30, 1)
FIELD(VFP_FPSCR, N, 31, 1)

FIELD(VFP_FPSCR, NZCV, 28, 4)
FIELD(VFP_FPSCR, NZCVQC, 27, 5)

/* v7M FPCCR bits */
FIELD(V7M_FPCCR, LSPACT, 0, 1)
FIELD(V7M_FPCCR, USER, 1, 1)
FIELD(V7M_FPCCR, S, 2, 1) /* not banked */
FIELD(V7M_FPCCR, THREAD, 3, 1)
FIELD(V7M_FPCCR, HFRDY, 4, 1) /* not banked */
FIELD(V7M_FPCCR, MMRDY, 5, 1)
FIELD(V7M_FPCCR, BFRDY, 6, 1)  /* not banked */
FIELD(V7M_FPCCR, SFRDY, 7, 1)  /* not banked */
FIELD(V7M_FPCCR, MONRDY, 8, 1) /* not banked */
FIELD(V7M_FPCCR, SPLIMVIOL, 9, 1)
FIELD(V7M_FPCCR, UFRDY, 10, 1)
FIELD(V7M_FPCCR, RES0, 11, 15)
FIELD(V7M_FPCCR, TS, 26, 1)        /* not banked */
FIELD(V7M_FPCCR, CLRONRETS, 27, 1) /* not banked */
FIELD(V7M_FPCCR, CLRONRET, 28, 1)  /* not banked */
FIELD(V7M_FPCCR, LSPENS, 29, 1)    /* not banked */
FIELD(V7M_FPCCR, LSPEN, 30, 1)     /* not banked */
FIELD(V7M_FPCCR, ASPEN, 31, 1)

/* v7M CONTROL bits */
FIELD(V7M_CONTROL, nPRIV, 0, 1)
FIELD(V7M_CONTROL, SPSEL, 1, 1)
FIELD(V7M_CONTROL, FPCA, 2, 1) /* not banked */
FIELD(V7M_CONTROL, SFPA, 3, 1) /* not banked */
FIELD(V7M_CONTROL, BTI_EN, 4, 1)
FIELD(V7M_CONTROL, UBTI_EN, 5, 1)
FIELD(V7M_CONTROL, PAC_EN, 6, 1)
FIELD(V7M_CONTROL, UPAC_EN, 7, 1)
/* Bits [31:8] are RES0 on Cortex-M */

/* V7M CCR bits */
FIELD(V7M_CCR, NONBASETHRDENA, 0, 1) /* RES1 on Armv8M */
FIELD(V7M_CCR, USERSETMPEND, 1, 1)
/* [2] RES0 */
FIELD(V7M_CCR, UNALIGN_TRP, 3, 1)
FIELD(V7M_CCR, DIV_0_TRP, 4, 1)
FIELD(V7M_CCR, BFHFNMIGN, 8, 1) /* not banked */
FIELD(V7M_CCR, STKALIGN, 9, 1)  /* RES1 on Armv8M */
FIELD(V7M_CCR, STKOFHFNMIGN, 10, 1)
/* [15:11] RES0 */
FIELD(V7M_CCR, DC, 16, 1)
FIELD(V7M_CCR, IC, 17, 1)
FIELD(V7M_CCR, BP, 18, 1)
FIELD(V7M_CCR, LOB, 19, 1)
FIELD(V7M_CCR, TRD, 20, 1) /* not banked */
/* [31:21] RES0 */

#define V7M_CCR_NOT_BANKED_FIELDS (FIELD_MASK(V7M_CCR, BFHFNMIGN) | FIELD_MASK(V7M_CCR, TRD))

#undef FIELD
