/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cpu.h"
#include "helper.h"
#include "system_registers.h"

#define ARM_ARCHITECTURE_MASK (0xFF00FFF0)

inline const char *sysreg_patch_lookup_name(const char *name)
{
    return name;
}

void HELPER(set_cp_reg)(CPUState *env, void *rip, uint32_t value)
{
    const ARMCPRegInfo *ri = rip;

    if(ri->type & ARM_CP_IO) {
        //  Use mutex if executed in parallel.
        ri->writefn(env, ri, value);
    } else {
        ri->writefn(env, ri, value);
    }
}

uint32_t HELPER(get_cp_reg)(CPUState *env, void *rip)
{
    const ARMCPRegInfo *ri = rip;
    uint32_t res;

    if(ri->type & ARM_CP_IO) {
        //  Use mutex if executed in parallel.
        res = ri->readfn(env, ri);
    } else {
        res = ri->readfn(env, ri);
    }

    return res;
}

void HELPER(set_cp_reg64)(CPUState *env, void *rip, uint64_t value)
{
    const ARMCPRegInfo *ri = rip;

    if(ri->type & ARM_CP_IO) {
        //  Use mutex if executed in parallel.
        ri->writefn(env, ri, value);
    } else {
        ri->writefn(env, ri, value);
    }
}

uint64_t HELPER(get_cp_reg64)(CPUState *env, void *rip)
{
    const ARMCPRegInfo *ri = rip;
    uint64_t res;

    if(ri->type & ARM_CP_IO) {
        //  Use mutex if executed in parallel.
        res = ri->readfn(env, ri);
    } else {
        res = ri->readfn(env, ri);
    }

    return res;
}

static inline uint32_t get_mpidr(CPUState *env)
{
    int mpidr = tlib_get_mp_index();
    /* We don't support setting cluster ID ([8..11])
     * so these bits always RAZ.
     */
    if(arm_feature(env, ARM_FEATURE_V7MP)) {
        mpidr |= (1 << 31);
        /* Cores which are uniprocessor (non-coherent)
         * but still implement the MP extensions set
         * bit 30. (For instance, A9UP.) However we do
         * not currently model any of those cores.
         */
    }
    return mpidr;
}
READ_FUNCTION(64, c0_mpidr, get_mpidr(env))

static inline void set_ttbcr(CPUState *env, uint64_t val)
{
    if(arm_feature(env, ARM_FEATURE_LPAE)) {
        env->cp15.c2_ttbcr_eae = (val >> 31) & 1;
    }
    val &= 7;
    env->cp15.c2_control = val;
    env->cp15.c2_mask = ~(((uint32_t)0xffffffffu) >> val);
    env->cp15.c2_base_mask = ~((uint32_t)0x3fffu >> val);
}
RW_FUNCTIONS(64, c2_ttbcr, env->cp15.c2_control, set_ttbcr(env, value))

static inline uint32_t get_ccsidr(CPUState *env)
{
    if(!arm_feature(env, ARM_FEATURE_V7)) {
        return 0;
    }

    uint32_t csselr = env->cp15.c0_cssel;
    uint32_t cache_level = (csselr >> 1) + 1;
    tlib_assert(cache_level <= 7);  //  Setting CSSELR to value over 15 shouldn't be possible.

    //  Bits 0-2 for L1, 3-5 for L2...
    uint32_t cache_type = extract32(env->cp15.c0_clid, 3 * (cache_level - 1), 3);

    enum {
        NO_CACHE = 0,
        ONLY_ICACHE = 1,
        ONLY_DCACHE = 2,
        SEPARATE_CACHES = 3,  //  Both caches are implemented.
        UNIFIED_CACHE = 4,
    };
    tlib_assert(cache_type <= UNIFIED_CACHE);  //  CLIDR is RO so we set it to invalid value if this fails.

    bool icache_accessed = csselr & 1u;  //  Data or unified cache is being accessed otherwise.
    bool cache_implemented =
        (icache_accessed ? cache_type == ONLY_ICACHE : (cache_type == ONLY_DCACHE || cache_type == UNIFIED_CACHE)) ||
        cache_type == SEPARATE_CACHES;

    if(cache_implemented) {
        return env->cp15.c0_ccsid[env->cp15.c0_cssel];
    } else {
        tlib_printf(LOG_LEVEL_WARNING,
                    "Tried to access CCSIDR with invalid CSSELR value: %u; L%u %s cache isn't implemented, returning 0", csselr,
                    cache_level, icache_accessed ? "instruction" : "data or unified");
        return 0x0;
    }
}
READ_FUNCTION(64, c0_ccsidr, get_ccsidr(env))

static inline uint32_t get_clidr(CPUState *env)
{
    if(!arm_feature(env, ARM_FEATURE_V7)) {
        return 0;
    }

    return env->cp15.c0_clid;
}
READ_FUNCTION(64, c0_clidr, get_clidr(env))

/* MMU TLB control.  */
/* Invalidate all.  */
WRITE_FUNCTION(64, invalidate_all, tlb_flush(env, 0, true));
/* Invalidate single TLB entry.  */
WRITE_FUNCTION(64, invalidate_single, tlb_flush_page(env, value &TARGET_PAGE_MASK, true));
/* Invalidate on ASID.  */
WRITE_FUNCTION(64, invalidate_on_asid, tlb_flush(env, value == 0, true));
/* Invalidate single entry on MVA.  */
/* ??? This is like case 1, but ignores ASID.  */
WRITE_FUNCTION(64, invalidate_single_on_mva, tlb_flush(env, 1, true));

static inline void set_c3(CPUState *env, uint64_t val)
{
    env->cp15.c3 = val;
    tlb_flush(env, 1, true); /* Flush TLB as domain not tracked in TLB */
}
RW_FUNCTIONS(64, c3, env->cp15.c3, set_c3(env, value))

//  From helper.c
uint32_t simple_mpu_ap_bits(uint32_t val);
uint32_t extended_mpu_ap_bits(uint32_t val);

static inline uint32_t get_c5_data(CPUState *env)
{
    if(arm_feature(env, ARM_FEATURE_PMSA)) {  //  DFSR
        return (env->cp15.c5_data & (MPU_FAULT_STATUS_BITS_FIELD_MASK | MPU_FAULT_STATUS_WRITE_FIELD_MASK));
    }
    if(arm_feature(env, ARM_FEATURE_MPU)) {
        return simple_mpu_ap_bits(env->cp15.c5_data);
    }
    return env->cp15.c5_data;
}
static inline void set_c5_data(CPUState *env, uint64_t val)
{
    if(arm_feature(env, ARM_FEATURE_MPU)) {
        val = extended_mpu_ap_bits(val);
    }
    env->cp15.c5_data = val;
}
RW_FUNCTIONS(64, c5_data, get_c5_data(env), set_c5_data(env, value))

static inline uint32_t get_c5_insn(CPUState *env)
{
    if(arm_feature(env, ARM_FEATURE_PMSA)) {  //  IFSR
        return (env->cp15.c5_insn & MPU_FAULT_STATUS_BITS_FIELD_MASK);
    }
    if(arm_feature(env, ARM_FEATURE_MPU)) {
        return simple_mpu_ap_bits(env->cp15.c5_data);
    }
    return env->cp15.c5_insn;
}
static inline void set_c5_insn(CPUState *env, uint64_t val)
{
    if(arm_feature(env, ARM_FEATURE_MPU)) {
        val = extended_mpu_ap_bits(val);
    }
    env->cp15.c5_insn = val;
}
RW_FUNCTIONS(64, c5_insn, get_c5_insn(env), set_c5_insn(env, value))

static inline void set_c13_context(CPUState *env, uint64_t val)
{
    /* This changes the ASID, so do a TLB flush.  */
    if(env->cp15.c13_context != val && !arm_feature(env, ARM_FEATURE_MPU)) {
        tlb_flush(env, 0, true);
    }
    env->cp15.c13_context = (uint32_t)val;
}
RW_FUNCTIONS(64, c13_context, env->cp15.c13_context, set_c13_context(env, value))

static inline void set_c15_i_max_min(CPUState *env)
{
    env->cp15.c15_i_max = 0x000;
    env->cp15.c15_i_min = 0xff0;
}
WRITE_FUNCTION(64, set_c15_i_max_min, set_c15_i_max_min(env))

static inline uint64_t get_c0_mpuir(CPUState *env)
{
    return (env->number_of_mpu_regions << MPU_TYPE_DREGION_FIELD_OFFSET) & MPU_TYPE_DREGION_FIELD_MASK;
}
READ_FUNCTION(64, c0_mpuir, get_c0_mpuir(env))

static inline void set_c5_csselr(CPUState *env, uint64_t val)
{
    env->cp15.c0_cssel = val & 0xf;

    if(unlikely(env->cp15.c0_cssel != val)) {
        tlib_printf(LOG_LEVEL_WARNING,
                    "Tried to set reserved bits by writing value to CSSELR: %" PRIu64 ", only bits 0-3 were set", val);
    }
}
RW_FUNCTIONS(64, c0_csselr, env->cp15.c0_cssel, set_c5_csselr(env, value))

static inline uint64_t get_c13_fcse(CPUState *env)
{
    return env->cp15.c13_fcse;
}
static inline void set_c13_fcse(CPUState *env, uint64_t val)
{
    /* Unlike real hardware the qemu TLB uses virtual addresses,
        not modified virtual addresses, so this causes a TLB flush.
     */
    if(env->cp15.c13_fcse != val) {
        tlb_flush(env, 1, true);
    }
    env->cp15.c13_fcse = val;
}
RW_FUNCTIONS(64, c13_fcse, get_c13_fcse(env), set_c13_fcse(env, value))

static inline void set_c7_par(CPUState *env, uint64_t val)
{
    if(arm_feature(env, ARM_FEATURE_VAPA)) {
        if(arm_feature(env, ARM_FEATURE_V7)) {
            env->cp15.c7_par = val & 0xfffff6ff;
        } else {
            env->cp15.c7_par = val & 0xfffff1ff;
        }
    }
}
RW_FUNCTIONS(64, c7_par, env->cp15.c7_par, set_c7_par(env, value))

static inline void ats1_helper(CPUState *env, uint64_t val, int is_user, int access_type)
{
    set_c15_i_max_min(env);

    uint32_t phys_addr = 0;
    target_ulong page_size;
    int prot;

    int ret = get_phys_addr(env, val, env->secure, access_type, is_user, &phys_addr, &prot, &page_size, 0);
    if(ret == 0) {
        /* We do not set any attribute bits in the PAR */
        if(page_size == (1 << 24) && arm_feature(env, ARM_FEATURE_V7)) {
            env->cp15.c7_par = (phys_addr & 0xff000000) | 1 << 1;
        } else {
            env->cp15.c7_par = phys_addr & 0xfffff000;
        }
    } else {
        env->cp15.c7_par = ((ret & (10 << 1)) >> 5) | ((ret & (12 << 1)) >> 6) | ((ret & 0xf) << 1) | 1;
    }
}

static inline void set_c7_ats1cpr(CPUState *env, uint64_t val)
{
    if(arm_feature(env, ARM_FEATURE_VAPA)) {
        ats1_helper(env, val, 0, 0);
    }
}
WRITE_FUNCTION(64, c7_ats1cpr, set_c7_ats1cpr(env, value))

static inline void set_c7_ats1cpw(CPUState *env, uint64_t val)
{
    if(arm_feature(env, ARM_FEATURE_VAPA)) {
        ats1_helper(env, val, 0, 1);
    }
}
WRITE_FUNCTION(64, c7_ats1cpw, set_c7_ats1cpw(env, value))

static inline void set_c7_ats1cur(CPUState *env, uint64_t val)
{
    if(arm_feature(env, ARM_FEATURE_VAPA)) {
        ats1_helper(env, val, 1, 0);
    }
}
WRITE_FUNCTION(64, c7_ats1cur, set_c7_ats1cur(env, value))

static inline void set_c7_ats1cuw(CPUState *env, uint64_t val)
{
    if(arm_feature(env, ARM_FEATURE_VAPA)) {
        ats1_helper(env, val, 1, 1);
    }
}
WRITE_FUNCTION(64, c7_ats1cuw, set_c7_ats1cuw(env, value))

//  Declarations of PMU (Performance Monitoring Unit)-related functions. Their impl. is in "pmu.c"
RW_FUNCTIONS(64, c9_pmcntenset, env->cp15.c9_pmcnten, set_c9_pmcntenset(env, value))
RW_FUNCTIONS(64, c9_pmcntenclr, env->cp15.c9_pmcnten, set_c9_pmcntenclr(env, value))
RW_FUNCTIONS(64, c9_pmcr, env->cp15.c9_pmcr, set_c9_pmcr(env, value))
RW_FUNCTIONS(64, c9_pmovsr, env->cp15.c9_pmovsr, set_c9_pmovsr(env, value))
RW_FUNCTIONS(64, c9_pmuserenr, env->cp15.c9_pmuserenr, set_c9_pmuserenr(env, value))
RW_FUNCTIONS(64, c9_pmintenset, env->cp15.c9_pminten, set_c9_pmintenset(env, value))
RW_FUNCTIONS(64, c9_pmintenclr, env->cp15.c9_pminten, set_c9_pmintenclr(env, value))
READ_FUNCTION(64, c9_pmccntr, get_c9_pmccntr(env))
RW_FUNCTIONS(64, c9_pmselr, env->pmu.selected_counter_id, set_c9_pmselr(env, value))
RW_FUNCTIONS(64, c9_pmxevtyper, env->cp15.c9_pmxevtyper, set_c9_pmxevtyper(env, value))
RW_FUNCTIONS(64, c9_pmxevcntr, get_c9_pmxevcntr(env), set_c9_pmxevcntr(env, value))
WRITE_FUNCTION(64, c9_pmswinc, set_c9_pmswinc(env, value))

static inline void set_c1_sctlr(CPUState *env, uint64_t val)
{
    env->cp15.c1_sys = val;
    /* ??? Lots of these bits are not implemented.  */
    /* This may enable/disable the MMU, so do a TLB flush.  */
    tlb_flush(env, 1, true);
}
RW_FUNCTIONS(64, c1_sctlr, env->cp15.c1_sys, set_c1_sctlr(env, value))

static inline void set_c1_cpacr(CPUState *env, uint64_t val)
{
    if(env->cp15.c1_coproc != val) {
        env->cp15.c1_coproc = val;
        /* We're ensuring end of the block with `ARM_CP_FORCE_TB_END` so it's safe to call from within a TB */
        tb_flush(env);
    }
}
RW_FUNCTIONS(64, c1_cpacr, env->cp15.c1_coproc, set_c1_cpacr(env, value))

static inline void set_c6_rgnr(CPUState *env, uint64_t val)
{
    if(val >= env->number_of_mpu_regions) {
        tlib_abortf("Region number %u doesn't point to a valid region", val);
    }
    env->cp15.c6_region_number = val;
}
RW_FUNCTIONS(64, c6_rgnr, env->cp15.c6_region_number, set_c6_rgnr(env, value))

static inline uint64_t get_c6_drbar(CPUState *env)
{
    return env->cp15.c6_base_address[env->cp15.c6_region_number];
}
static inline void set_c6_drbar(CPUState *env, uint64_t val)
{
    if(val & 0b11111) {
        //  ISA requires address to be divisible by 4, but due to current MPU implementation it also has to be divisible by 32
        tlib_abortf("Region size smaller than 32 bytes is not supported. Region base address must be divisible by 32");
    }
    env->cp15.c6_base_address[env->cp15.c6_region_number] = val;
    tlb_flush(env, 1, false);
}
RW_FUNCTIONS(64, c6_drbar, get_c6_drbar(env), set_c6_drbar(env, value))

static inline uint64_t get_c6_drsr(CPUState *env)
{
    const uint32_t index = env->cp15.c6_region_number;

    return env->cp15.c6_size_and_enable[index] | (env->cp15.c6_subregion_disable[index] << 8);
}
static inline void set_c6_drsr(CPUState *env, uint64_t val)
{
    const uint32_t index = env->cp15.c6_region_number;

    env->cp15.c6_size_and_enable[index] = val & MPU_SIZE_AND_ENABLE_FIELD_MASK;
    env->cp15.c6_subregion_disable[index] = (val & MPU_SUBREGION_DISABLE_FIELD_MASK) >> MPU_SUBREGION_DISABLE_FIELD_OFFSET;
    tlb_flush(env, 1, false);
}
RW_FUNCTIONS(64, c6_drsr, get_c6_drsr(env), set_c6_drsr(env, value))

static inline uint64_t get_c6_dracr(CPUState *env)
{
    return env->cp15.c6_access_control[env->cp15.c6_region_number];
}
static inline void set_c6_dracr(CPUState *env, uint64_t val)
{
    env->cp15.c6_access_control[env->cp15.c6_region_number] = val;
    tlb_flush(env, 1, false);
}
RW_FUNCTIONS(64, c6_dracr, get_c6_dracr(env), set_c6_dracr(env, value))

static inline uint64_t get_c1_actlr(CPUState *env, const ARMCPRegInfo *info)
{
    switch(ARM_CPUID(env) & ARM_ARCHITECTURE_MASK) {
        case ARM_CPUID_ARM1026EJ_S &ARM_ARCHITECTURE_MASK:
            return 1;
        case ARM_CPUID_ARM1136JF_S &ARM_ARCHITECTURE_MASK:
        case ARM_CPUID_ARM1176JZF_S &ARM_ARCHITECTURE_MASK:
            return 7;
        case ARM_CPUID_ARM11MPCORE &ARM_ARCHITECTURE_MASK:
            return 1;
        case ARM_CPUID_CORTEXA5 &ARM_ARCHITECTURE_MASK:
            return 0;
        case ARM_CPUID_CORTEXA7 &ARM_ARCHITECTURE_MASK:
            return 0;
        case ARM_CPUID_CORTEXA8 &ARM_ARCHITECTURE_MASK:
            return 2;
        case ARM_CPUID_CORTEXA9 &ARM_ARCHITECTURE_MASK:
            return 0;
        case ARM_CPUID_CORTEXA15 &ARM_ARCHITECTURE_MASK:
            return 0;
        default:
            return tlib_read_cp15_32(encode_as_aarch32_32bit_register(info));
    }
}
RW_FUNCTIONS(64, c1_actlr, get_c1_actlr(env, info), tlib_write_cp15_32(encode_as_aarch32_32bit_register(info), value))

RW_FUNCTIONS(64, c10_tlb_lockdown, 0, tlib_write_cp15_32(encode_as_aarch32_32bit_register(info), value))

WRITE_FUNCTION(64, write_cp15, tlib_write_cp15_32(encode_as_aarch32_32bit_register(info), value))
RW_FUNCTIONS(64, read_write_cp15, tlib_read_cp15_32(encode_as_aarch32_32bit_register(info)),
             tlib_write_cp15_32(encode_as_aarch32_32bit_register(info), value))

static inline uint64_t get_c9_l2auxcctrl(CPUState *env)
{
    /* L2 cache auxiliary control (A8) or control (A15) */
    if(ARM_CPUID(env) == ARM_CPUID_CORTEXA15) {
/* Linux wants the number of processors from here.
 * Might as well set the interrupt-controller bit too.
 */
#define smp_cpus 1  //  TODO: should return correct number of cpus
        return ((smp_cpus - 1) << 24) | (1 << 23);
    }
    return 0;
}
RW_FUNCTIONS(64, c9_l2auxcctrl, get_c9_l2auxcctrl(env), return)

static inline void set_c15_cpar(CPUState *env, uint64_t val)
{
    if(env->cp15.c15_cpar != (val & 0x3fff)) {
        /* Changes cp0 to cp13 behavior, so needs a TB flush.  */
        tb_flush(env);
        env->cp15.c15_cpar = val & 0x3fff;
    }
}
RW_FUNCTIONS(64, c15_cpar, env->cp15.c15_cpar, set_c15_cpar(env, value))

RW_FUNCTIONS(64, c15_threadid, env->cp15.c15_threadid, env->cp15.c15_threadid = value & 0xffff)

static inline void set_c15_ticonfig(CPUState *env, uint64_t val)
{
    env->cp15.c15_ticonfig = val & 0xe7;
    env->cp15.c0_cpuid = (val & (1 << 5)) ? /* OS_TYPE bit */
                             ARM_CPUID_TI915T
                                          : ARM_CPUID_TI925T;
}
RW_FUNCTIONS(64, c15_ticonfig, env->cp15.c15_ticonfig, set_c15_ticonfig(env, value))

static inline uint64_t get_c9_tcmregion(CPUState *env, int op2)
{
    return env->cp15.c9_tcmregion[op2][env->cp15.c9_tcmsel];
}
static inline void set_c9_tcmregion(CPUState *env, int op2, uint64_t val)
{
    uint32_t tcm_region_index = env->cp15.c9_tcmsel;
    uint32_t tcm_region_value = env->cp15.c9_tcmregion[op2][tcm_region_index];
    if(val != tcm_region_value) {
        tlib_abortf("Attempted to change TCM region #%u for interface #%u from 0x%08x to 0x%08x, reconfiguration at runtime is "
                    "currently not supported",
                    tcm_region_index, op2, tcm_region_value, val);
    }
}
RW_FUNCTIONS(64, c9_tcmregion_0, get_c9_tcmregion(env, 0), set_c9_tcmregion(env, 0, value))
RW_FUNCTIONS(64, c9_tcmregion_1, get_c9_tcmregion(env, 1), set_c9_tcmregion(env, 1, value))

static inline void set_c9_tcmsel(CPUState *env, uint64_t val)
{
    if(val >= MAX_TCM_REGIONS) {
        tlib_abortf("Attempted access to TCM region #%u, maximal supported value is %u", val, MAX_TCM_REGIONS);
    }
    env->cp15.c9_tcmsel = val;
}
RW_FUNCTIONS(64, c9_tcmsel, env->cp15.c9_tcmsel, set_c9_tcmsel(env, value))

static inline void set_cp14_c6_teecr(CPUState *env, uint64_t val)
{
    helper_set_teecr(env, val);
}
RW_FUNCTIONS(64, cp14_c6_teecr, env->teecr, set_cp14_c6_teecr(env, value))

RW_FUNCTIONS(64, c12_vbar, env->cp15.c12_vbar, env->cp15.c12_vbar = value & ~0xF)

#define CREATE_FEATURE_REG(name, op2) \
    ARM32_CP_REG_DEFINE(name, 15, 0, 0, 1, op2, 1, RW, FIELD(cp15.c0_c1[op2]))  //  Processor Feature Register [op2]

#define CREATE_ISAR_FEATURE_REG(name, op2) \
    ARM32_CP_REG_DEFINE(name, 15, 0, 0, 2, op2, 1, RW, FIELD(cp15.c0_c2[op2]))  //  ISA Feature Register [op2]

#define READ_AS_ZERO(cp, op1, crn, crm, op2, el) \
    ARM32_CP_REG_DEFINE(ZERO, cp, op1, crn, crm, op2, el, RO | CONST(0))  //  Marked as Read-As-Zero in docs

// clang-format off
static ARMCPRegInfo general_coprocessor_registers[] = {
    // ================== Coprocessor 14 ==================
    // The params are:  name              cp, op1, crn, crm, op2,  el,  extra_type, ...

    /* Minimal set of debug CP14 registers is implemented, since we don't support debug */
    /* DBGDIDR: just RAZ. In particular this means the
     * "debug architecture version" bits will read as
     * a reserved value, which should cause Linux to
     * not try to use the debug hardware.
     */
    ARM32_CP_REG_DEFINE(DBGDIDR,          14,   0,   0,   0,   0,   0,  RO | CONST(0)) // Debug ID Register

    // ================== Coprocessor 15 ==================
    // crn == 0
    // The params are:  name              cp, op1, crn, crm, op2,  el,  extra_type, ...
    ARM32_CP_REG_DEFINE(MIDR,             15,   0,   0,   0,   0,   1,  RO, FIELD(cp15.c0_cpuid))                      // Main ID Register
    ARM32_CP_REG_DEFINE(CTR,              15,   0,   0,   0,   1,   1,  RO, FIELD(cp15.c0_cachetype))                  // Cache Type Register

    ARM32_CP_REG_DEFINE(TCMTR,            15,   0,   0,   0,   2,   1,  RO, FIELD(cp15.c0_tcmtype))                    // TCMTR, TCM Type Register, TCM status
    ARM32_CP_REG_DEFINE(TLBTR,            15,   0,   0,   0,   3,   1,  RO | CONST(0)) /* No lockable TLB entries.  */ // TLBTR, TLB Type Register

    // crm == 3..7, opc2 == 0..7
    READ_AS_ZERO(15, 0, 0, 3, ANY, 1)
    READ_AS_ZERO(15, 0, 0, 4, ANY, 1)
    READ_AS_ZERO(15, 0, 0, 5, ANY, 1)
    READ_AS_ZERO(15, 0, 0, 6, ANY, 1)
    READ_AS_ZERO(15, 0, 0, 7, ANY, 1)

    // crn == 3
    // The params are:  name              cp, op1, crn, crm, op2,  el,  extra_type, ...
    ARM32_CP_REG_DEFINE(C3,               15, ANY,   3, ANY, ANY,   1,  RW, RW_FNS(c3))                 // MMU Domain access control (DACR) / MPU write buffer control

    // crn == 5
    ARM32_CP_REG_DEFINE(DFSR,             15,   0,   5,   0,   0,   1,  RW, RW_FNS(c5_data))        // Data Fault Status Register
    ARM32_CP_REG_DEFINE(IFSR,             15,   0,   5,   0,   1,   1,  RW, RW_FNS(c5_insn))        // Instruction Fault Status Register

    // crn == 7
    // The params are:  name           cp, op1, crn, crm, op2,  el,  extra_type, ...
    ARM32_CP_REG_DEFINE(ICIALLU,       15,   0,   7,   5,   0,   1,  WO | IGNORED) // Instruction Cache Invalidate All to PoU
    ARM32_CP_REG_DEFINE(ICIMVAU,       15,   0,   7,   5,   1,   1,  WO | IGNORED) // Instruction Cache line Invalidate by VA to PoU
    ARM32_CP_REG_DEFINE(BPIALL,        15,   0,   7,   5,   6,   1,  WO | IGNORED) // Branch Predictor Invalidate All
    ARM32_CP_REG_DEFINE(BPIMVA,        15,   0,   7,   5,   7,   1,  WO | IGNORED) // Branch Predictor Invalidate by VA
    ARM32_CP_REG_DEFINE(DCIMVAC,       15,   0,   7,   6,   1,   1,  WO | IGNORED) // Data Cache line Invalidate by MVA to PoC
    ARM32_CP_REG_DEFINE(DCISW,         15,   0,   7,   6,   2,   1,  WO | IGNORED) // Data Cache line Invalidate by Set/Way
    ARM32_CP_REG_DEFINE(DCCMVAC,       15,   0,   7,  10,   1,   1,  WO | IGNORED) // Data Cache line Clean by VA to PoC
    ARM32_CP_REG_DEFINE(DCCSW,         15,   0,   7,  10,   2,   1,  WO | IGNORED) // Data Cache line Clean by Set/Way
    ARM32_CP_REG_DEFINE(DCCMVAU,       15,   0,   7,  11,   1,   1,  WO | IGNORED) // Data Cache line Clean by VA to PoU
    ARM32_CP_REG_DEFINE(DCCIMVAC,      15,   0,   7,  14,   1,   1,  WO | IGNORED) // Data Cache line Clean and Invalidate by VA to PoC
    ARM32_CP_REG_DEFINE(DCCISW,        15,   0,   7,  14,   2,   1,  WO | IGNORED) // Data Cache line Clean and Invalidate by Set/Way

    ARM32_CP_REG_DEFINE(PREICL,        15,   0,   7,  13,   1,   1,  WO | IGNORED) // Prefetch instruction cache line (ARMv5)
    ARM32_CP_REG_DEFINE(INVIDC,        15,   0,   7,   7,   0,   1,  WO | IGNORED) // Invalidate both instruction and data caches or unified cache (ARMv5)
    ARM32_CP_REG_DEFINE(INVUCL,        15,   0,   7,   7,   1,   1,  WO | IGNORED) // Invalidate unified cache line Set/way MVA (ARMv5)
    ARM32_CP_REG_DEFINE(INVICLSW,      15,   0,   7,   7,   2,   1,  WO | IGNORED) // Invalidate unified cache line Set/way (ARMv5)

    ARM32_CP_REG_DEFINE(PAR,           15,   0,   7,   4,   0,   1,  RW, RW_FNS(c7_par))             // Physical Address Register
    ARM32_CP_REG_DEFINE(ATS1CPR,       15,   0,   7,   8,   0,   1,  WO, WRITEFN(c7_ats1cpr))        // PL1 read translation
    ARM32_CP_REG_DEFINE(ATS1CPW,       15,   0,   7,   8,   1,   1,  WO, WRITEFN(c7_ats1cpw))        // PL1 write translation
    ARM32_CP_REG_DEFINE(ATS1CUW,       15,   0,   7,   8,   3,   1,  WO, WRITEFN(c7_ats1cuw))        // Unprivileged write translation

    // crn == 8
    // The params are:  name           cp, op1, crn, crm, op2,  el,  extra_type, ...
    ARM32_CP_REG_DEFINE(ITLBIALL,      15,   0,   8,   5,   0,   1,  WO, WRITEFN(invalidate_all))
    ARM32_CP_REG_DEFINE(ITLBIMVA,      15,   0,   8,   5,   1,   1,  WO, WRITEFN(invalidate_single))
    ARM32_CP_REG_DEFINE(ITLBIASID,     15,   0,   8,   5,   2,   1,  WO, WRITEFN(invalidate_on_asid))

    ARM32_CP_REG_DEFINE(DTLBIALL,      15,   0,   8,   6,   0,   1,  WO, WRITEFN(invalidate_all))
    ARM32_CP_REG_DEFINE(DTLBIMVA,      15,   0,   8,   6,   1,   1,  WO, WRITEFN(invalidate_single))
    ARM32_CP_REG_DEFINE(DTLBIASID,     15,   0,   8,   6,   2,   1,  WO, WRITEFN(invalidate_on_asid))

    ARM32_CP_REG_DEFINE(TLBIALL,       15,   0,   8,   7,   0,   1,  WO, WRITEFN(invalidate_all))
    ARM32_CP_REG_DEFINE(TLBIMVA,       15,   0,   8,   7,   1,   1,  WO, WRITEFN(invalidate_single))
    ARM32_CP_REG_DEFINE(TLBIASID,      15,   0,   8,   7,   2,   1,  WO, WRITEFN(invalidate_on_asid))

    // crn == 9
    ARM32_CP_REG_DEFINE(L1_C9DATA,     15,   0,   9,   0,   0,   1,  RW, FIELD(cp15.c9_data))      // L1 Cache lockdown
    ARM32_CP_REG_DEFINE(L1_C9INSN,     15,   0,   9,   0,   1,   1,  RW, FIELD(cp15.c9_insn))      // L1 Cache lockdown

    ARM32_CP_REG_DEFINE(L2LOCKDOWN,    15,   1,   9,   0,   0,   1,  RW | CONST(0))                // L2 Cache lockdown (A8 only)
    ARM32_CP_REG_DEFINE(L2AUXCCTRL,    15,   1,   9,   0,   2,   1,  RW, RW_FNS(c9_l2auxcctrl))    // L2 Cache auxiliary control (A8) or control (A15)
    ARM32_CP_REG_DEFINE(L2EXCTRL,      15,   1,   9,   0,   3,   1,  RW | CONST(0))                // L2 Cache extended control (A15)

    ARM32_CP_REG_DEFINE(DTCMRR,        15,   0,   9,   1,   0,   1,  RW, RW_FNS(c9_tcmregion_0))   // Data TCM Region Register (also BTCM)
    ARM32_CP_REG_DEFINE(ITCMRR,        15,   0,   9,   1,   1,   1,  RW, RW_FNS(c9_tcmregion_1))   // Instruction or unified TCM Region Register (also ATCM)

    ARM32_CP_REG_DEFINE(DTCM-NSACR,    15,   0,   9,   1,   2,   1,  RW)                           // Data TCM Non-Secure Access Control Register
    ARM32_CP_REG_DEFINE(ITCM-NSACR,    15,   0,   9,   1,   3,   1,  RW)                           // Instruction TCM Non-Secure Access Control Register

    // Cortex-R8 doesn't have it.
    ARM32_CP_REG_DEFINE(TCMSR,         15,   0,   9,   2,   0,   1,  RW, RW_FNS(c9_tcmsel))        // TCM Selection Register

    // crn == 10
    ARM32_CP_REG_DEFINE(TLB_LOCKDOWN,  15, ANY,  10, ANY, ANY,   1,  RW, RW_FNS(c10_tlb_lockdown)) // MMU TLB lockdown

    // crn == 13
    ARM32_CP_REG_DEFINE(FCSEIDR,       15,   0,  13,   0,   0,   1,  RW, RW_FNS(c13_fcse))         // FCSE PID Register
    ARM32_CP_REG_DEFINE(CONTEXTIDR,    15,   0,  13,   0,   1,   1,  RW, RW_FNS(c13_context))      // Context ID Register
};

static ARMCPRegInfo sctlr_register[] = {
    // crn == 1
    /* Normally we would always end the TB after register write, but Linux
     * arch/arm/mach-pxa/sleep.S expects two instructions following
     * an MMU enable to execute from cache. Imitate this behaviour.  */
    ARM32_CP_REG_DEFINE(SCTLR,            15,   0,   1,   0,   0,   1,  RW | ARM_CP_SUPPRESS_TB_END, RW_FNS(c1_sctlr)) // System Control Register
};

static ARMCPRegInfo feature_v7_registers[] = {
    // ================== Coprocessor 14 ==================
    // The params are:  name              cp, op1, crn, crm, op2,  el,  extra_type, ...
    /* DBGDRAR and DBGDSAR: v7 only. Always RAZ since we
     * don't implement memory mapped debug components
     */

    ARM32_CP_REG_DEFINE(DBGDSCRint,       14,   0,   0,   1,   0,   0,  RO | CONST(0)) // Debug Status and Control internal
    ARM32_CP_REG_DEFINE(DBGDSCRext,       14,   0,   0,   2,   2,   0,  RO | CONST(0)) // Debug Status and Control external
    ARM32_CP_REG_DEFINE(DBGDRAR,          14,   0,   1,   0,   0,   0,  RO, FIELD(cp14.c1_dbgdrar)) // Debug ROM Address Register
    ARM32_CP_REG_DEFINE(DBGDSAR,          14,   0,   2,   0,   0,   0,  RO, FIELD(cp14.c2_dbgdsar)) // Debug Self Address Offset Register

    // ================== Coprocessor 15 ==================
    // The params are:  name              cp, op1, crn, crm, op2,  el, extra_type, ...
    ARM32_CP_REG_DEFINE(CSSELR,           15,   2,   0,   0,   0,   1, RW, RW_FNS(c0_csselr)) // Cache Size Selection Register

    // REVIDR register is an additional minor revision information which isn't specified even
    // in the specific CPU model's manual (e.g. Cortex-R8). It's a MIDR alias if not implemented
    // which is why the `cp15.c0_cpuid` field provides a value to it.
    ARM32_CP_REG_DEFINE(REVIDR,           15,   0,   0,   0,   6,   1, RO, FIELD(cp15.c0_cpuid)) // Revision ID Register

    ARM32_CP_REG_DEFINE(ADFSR,            15,   0,   5,   1,   0,   1, RW)                    // Auxiliary Data Fault Status Register
    ARM32_CP_REG_DEFINE(AIFSR,            15,   0,   5,   1,   1,   1, RW)                    // Auxiliary Instruction Fault Status Register

    ARM32_CP_REG_DEFINE(CP15WFIprev7,     15,   0,   7,   0,   4,   1, WO | ARM_CP_NOP)       // Wait For Interrupt pre-v7, now NOP

    // Performance Monitor Extensions
    /* We force TB end on some of these registers, even on reads
     * The reason is that PMU can be used to count instructions and it takes instruction count value from the TB header (block_header_arch_action)
     * So we do this to increase the precision of the counters
     */
    ARM32_CP_REG_DEFINE(PMCR,             15,   0,   9,  12,   0,   0, RW | ARM_CP_FORCE_TB_END, RW_FNS(c9_pmcr))       // Performance monitor control register
    ARM32_CP_REG_DEFINE(PMCNTENSET,       15,   0,   9,  12,   1,   0, RW | ARM_CP_FORCE_TB_END, RW_FNS(c9_pmcntenset)) // Performance monitor Count enable set register
    ARM32_CP_REG_DEFINE(PMCNTENCLR,       15,   0,   9,  12,   2,   0, RW | ARM_CP_FORCE_TB_END, RW_FNS(c9_pmcntenclr)) // Performance monitor Count enable clear
    ARM32_CP_REG_DEFINE(PMOVSR,           15,   0,   9,  12,   3,   0, RW, RW_FNS(c9_pmovsr))                           // Performance monitor Overflow flag status
    ARM32_CP_REG_DEFINE(PMSWINC,          15,   0,   9,  12,   4,   0, WO, WRITEFN(c9_pmswinc))                         // Performance monitor software increment

    ARM32_CP_REG_DEFINE(PMSELR,           15,   0,   9,  12,   5,   0, RW, RW_FNS(c9_pmselr))                           // Performance monitor event counter selection register
    ARM32_CP_REG_DEFINE(PMCEID0,          15,   0,   9,  12,   6,   0, RO, FIELD(cp15.c9_pmceid0))                      // Performance Monitors Common Event ID registers 0
    ARM32_CP_REG_DEFINE(PMCEID1,          15,   0,   9,  12,   7,   0, RO | CONST(0))                                   // Performance Monitors Common Event ID registers 1

    ARM32_CP_REG_DEFINE(PMCCNTR,          15,   0,   9,  13,   0,   0, RO | ARM_CP_FORCE_TB_END, READFN(c9_pmccntr))    // Cycle counter register
    ARM32_CP_REG_DEFINE(PMXEVTYPER,       15,   0,   9,  13,   1,   0, RW | ARM_CP_FORCE_TB_END, RW_FNS(c9_pmxevtyper)) // Event type select
    ARM32_CP_REG_DEFINE(PMXEVCNTR,        15,   0,   9,  13,   2,   0, RW | ARM_CP_FORCE_TB_END, RW_FNS(c9_pmxevcntr))  // Event counter value get/set

    ARM32_CP_REG_DEFINE(PMUSERENR,        15,   0,   9,  14,   0,   0, RW | ARM_CP_FORCE_TB_END, RW_FNS(c9_pmuserenr))                        // Performance monitor control user access enable
    ARM32_CP_REG_DEFINE(PMINTENSET,       15,   0,   9,  14,   1,   1, RW, RW_FNS(c9_pmintenset))                       // Performance monitor control interrupt enable set
    ARM32_CP_REG_DEFINE(PMINTENCLR,       15,   0,   9,  14,   2,   1, RW, RW_FNS(c9_pmintenclr))                       // Performance monitor control interrupt enable clear

    // This one is only present in VMSA with Virtualization Extensions but this tlib doesn't support Virtualization Extensions.
    // Let's keep it commented-out to document its presence.
    //ARM32_CP_REG_DEFINE(PMOVSSET,         15,   0,   9,  14,   3,   1, RW)                                              // Performance Monitors Overflow Flag Status Set register
};

static ARMCPRegInfo feature_pre_v7_registers[] = {
    /* 0,c7,c0,4: Standard v6 WFI (also used in some pre-v6 cores).
     * In v7, this must NOP.
     */
    ARM32_CP_REG_DEFINE(CP15WFIprev7,  15,   0,   7,   0,   4,   1,  WO | ARM_CP_WFI)  // Wait For Interrupt

    // According to ARMv5 spec these should set ZF flag, when data cache is cleared.
    // The flag is set by coproc handling logic when destination register is r15 (PC) in do_coproc_insn
    ARM32_CP_REG_DEFINE(DCTCINV,       15,   0,   7,  14,   3,   1,  RO | CONST(CPSR_Z))    // Data Cache Test, Clean and Invalidate
    ARM32_CP_REG_DEFINE(DCTC,          15,   0,   7,  10,   3,   1,  RO | CONST(CPSR_Z))    // Data Cache Test and Clean
};

static ARMCPRegInfo feature_v6_registers[] = {
    // crn == 0, op1 == 0, crm == 1, op2 == 0..7
    CREATE_FEATURE_REG(ID_PFR0, 0)
    CREATE_FEATURE_REG(ID_PFR1, 1)
    CREATE_FEATURE_REG(ID_DFR0, 2)
    CREATE_FEATURE_REG(ID_AFR0, 3)
    CREATE_FEATURE_REG(ID_MMFR0, 4)
    CREATE_FEATURE_REG(ID_MMFR1, 5)
    CREATE_FEATURE_REG(ID_MMFR2, 6)
    CREATE_FEATURE_REG(ID_MMFR3, 7)
    // crn == 0, op1 == 0, crm == 2, op2 == 0..5
    CREATE_ISAR_FEATURE_REG(ID_ISAR0, 0)
    CREATE_ISAR_FEATURE_REG(ID_ISAR1, 1)
    CREATE_ISAR_FEATURE_REG(ID_ISAR2, 2)
    CREATE_ISAR_FEATURE_REG(ID_ISAR3, 3)
    CREATE_ISAR_FEATURE_REG(ID_ISAR4, 4)
    CREATE_ISAR_FEATURE_REG(ID_ISAR5, 5)
    CREATE_ISAR_FEATURE_REG(ID_ISAR_RESERVED6, 6)
    CREATE_ISAR_FEATURE_REG(ID_ISAR_RESERVED7, 7)

    // The params are:  name      cp, op1, crn, crm, op2,  el,  extra_type, ...
    ARM32_CP_REG_DEFINE(CCSIDR,   15,   1,   0,   0,   0,   1,  RO, READFN(c0_ccsidr))   // Cache Size ID Register
    ARM32_CP_REG_DEFINE(CLIDR,    15,   1,   0,   0,   1,   1,  RO, READFN(c0_clidr))    // Cache Level ID Register

    // It used to be CP15WFIprev6
    ARM32_CP_REG_DEFINE(ATS1CUR,  15,   0,   7,   8,   2,   1,  WO, WRITEFN(c7_ats1cur)) // Unprivileged read translation

    ARM32_CP_REG_DEFINE(CP15ISB,  15,   0,   7,   5,   4,   0,  WO | ARM_CP_BARRIER)     // Instruction Synchronization Barrier System instruction
    ARM32_CP_REG_DEFINE(CP15DSB,  15,   0,   7,  10,   4,   0,  WO | ARM_CP_BARRIER)     // Data Synchronization Barrier System instruction
    ARM32_CP_REG_DEFINE(CP15DMB,  15,   0,   7,  10,   5,   0,  WO | ARM_CP_BARRIER)     // Data Memory Barrier System instruction
};

static ARMCPRegInfo feature_pre_v6_registers[] = {
    /* 0,c7,c8,2: Not all pre-v6 cores implemented this WFI,
     * so this is slightly over-broad.
     */
    // The params are:  name           cp, op1, crn, crm, op2,  el,  extra_type, ...
    // Superseded by ATS1CUR
    ARM32_CP_REG_DEFINE(CP15WFIprev6,  15,   0,   7,   8,   2,   1,  WO | ARM_CP_WFI)     // Wait For Interrupt

    // This is CP15DSB on newer ISA
    ARM32_CP_REG_DEFINE(CP15DWB,       15,   0,   7,  10,   4,   0,  WO | ARM_CP_BARRIER) // Drain Write Buffer
};

static ARMCPRegInfo mpidr_register[] = {
    // The params are:  name          cp, op1, crn, crm, op2,  el,  extra_type, ...
    ARM32_CP_REG_DEFINE(MPIDR,        15,   0,   0,   0,   5,   1,  RO, READFN(c0_mpidr)) // Multiprocessor Affinity Register
};

static ARMCPRegInfo feature_mpu_registers[] = {
    // The params are:  name            cp, op1, crn, crm, op2,  el,  extra_type, ...
    ARM32_CP_REG_DEFINE(MPU_DATA,       15,   0,   2,   0,   0,   1,  RW, FIELD(cp15.c2_data))
    ARM32_CP_REG_DEFINE(MPU_INSN,       15,   0,   2,   0,   1,   1,  RW, FIELD(cp15.c2_insn))
};

static ARMCPRegInfo has_mpu_fault_addr_register[] = {
    // The params are:  name            cp, op1, crn, crm, op2,  el,  extra_type, ...
    /* MPU Fault Address */
    ARM32_CP_REG_DEFINE(MPU_FAULT_ADDR, 15, ANY,   6,   0, ANY,   1,  RW, FIELD(cp15.c6_addr))
    ARM32_CP_REG_DEFINE(MPU_FAULT_ADDR, 15, ANY,   6,   1, ANY,   1,  RW, FIELD(cp15.c6_addr))
    ARM32_CP_REG_DEFINE(MPU_FAULT_ADDR, 15, ANY,   6,   2, ANY,   1,  RW, FIELD(cp15.c6_addr))
    ARM32_CP_REG_DEFINE(MPU_FAULT_ADDR, 15, ANY,   6,   3, ANY,   1,  RW, FIELD(cp15.c6_addr))
    ARM32_CP_REG_DEFINE(MPU_FAULT_ADDR, 15, ANY,   6,   4, ANY,   1,  RW, FIELD(cp15.c6_addr))
    ARM32_CP_REG_DEFINE(MPU_FAULT_ADDR, 15, ANY,   6,   5, ANY,   1,  RW, FIELD(cp15.c6_addr))
    ARM32_CP_REG_DEFINE(MPU_FAULT_ADDR, 15, ANY,   6,   6, ANY,   1,  RW, FIELD(cp15.c6_addr))
    ARM32_CP_REG_DEFINE(MPU_FAULT_ADDR, 15, ANY,   6,   7, ANY,   1,  RW, FIELD(cp15.c6_addr))
};

static ARMCPRegInfo has_mmu_registers[] = {
    // The params are:  name              cp, op1, crn, crm, op2,  el,  extra_type, ...
    ARM32_CP_REG_DEFINE(TTBR0,            15,   0,   2,   0,   0,   1,  RW, FIELD(cp15.c2_base0)) // Translation Table Base Register 0
    ARM32_CP_REG_DEFINE(TTBR1,            15,   0,   2,   0,   1,   1,  RW, FIELD(cp15.c2_base1)) // Translation Table Base Register 1
    ARM32_CP_REG_DEFINE(TTBCR,            15,   0,   2,   0,   2,   1,  RW, RW_FNS(c2_ttbcr))     // Translation Table Base Control Register

    ARM32_CP_REG_DEFINE(DFAR,             15,   0,   6,   0,   0,   1,  RW, FIELD(cp15.c6_data))  // DFAR, Data Fault Address Register
    // Note that in WFAR we use the same address as in IFAR. This reg probably shouldn't exist in ISA newer then ARMv5
    ARM32_CP_REG_DEFINE(WFAR,             15,   0,   6,   0,   1,   1,  RW, FIELD(cp15.c6_insn))  // WFAR, Watchpoint Fault Address Register
    ARM32_CP_REG_DEFINE(IFAR,             15,   0,   6,   0,   2,   1,  RW, FIELD(cp15.c6_insn))  // IFAR, Instruction Fault Address Register

    // LPAE (64 bit variants)             cp, op1,      crm,       el,  extra_type, ...
    ARM32_CP_64BIT_REG_DEFINE(TTBR0_EA,   15,   0,        2,        1,  RW, FIELD(cp15.c2_base0_ea))
};

static ARMCPRegInfo has_cp15_c13_registers[] = {
    // The params are:  name              cp, op1, crn, crm, op2,  el, extra_type, ...
    ARM32_CP_REG_DEFINE(TPIDRPRW,         15,   0,  13,   0,   4,   1, RW, FIELD(cp15.c13_tls3))  // PL1 Software Thread ID Register
    // This should be read-only on PL0 and RW on PL1 - we do this in `cp15_special_user_ok`
    ARM32_CP_REG_DEFINE(TPIDRURO,         15,   0,  13,   0,   3,   0, RW, FIELD(cp15.c13_tls2))  // PL0 Read-Only Software Thread ID Register
    ARM32_CP_REG_DEFINE(TPIDRURW,         15,   0,  13,   0,   2,   0, RW, FIELD(cp15.c13_tls1))  // PL0 Read/Write Software Thread ID Register
    // if any other registers are needed, there should probably implemented as NOPs
};

static ARMCPRegInfo has_cp15_c13_dummy_registers[] = {
    // The params are:  name              cp, op1, crn, crm, op2,  el, extra_type, ...
    ARM32_CP_REG_DEFINE(TPIDRPRW,         15,   0,  13,   0,   4,   1, RO | CONST(0))  // PL1 Software Thread ID Register
    ARM32_CP_REG_DEFINE(TPIDRURO,         15,   0,  13,   0,   3,   0, RO | CONST(0))  // PL0 Read-Only Software Thread ID Register
    ARM32_CP_REG_DEFINE(TPIDRURW,         15,   0,  13,   0,   2,   0, RO | CONST(0))  // PL0 Read/Write Software Thread ID Register
};

// Some implementation details are handled by `do_coproc_insn_quirks`
static ARMCPRegInfo omap_registers[] = {
    // These registers will be cloned through all the crn=0 space but we can't do that as we would override exising r/w regs
    // This will be handled in `coproc_quirks` - we hack a little and put the register at unused encoding, where we will redirect all writes
    // It should be fine, as long as we don't jump out of this translation library via tlib_read/write_cp15
    // The params are:  name              cp, op1, crn, crm, op2,  el, extra_type, ...
    ARM32_CP_REG_DEFINE(OMAP_C0_DUMMY,    15,  10,   0,  10,  10,   1, WO | ARM_CP_NOP)  // In OMAP or XSCALE writes to crn0 have no effect, but don't raise exception either
    ARM32_CP_REG_DEFINE(OMAP_C9_DUMMY,    15,  10,   9,  10,  10,   1, RW | CONST(0))    // a similar hack to the one above
    ARM32_CP_REG_DEFINE(OMAP_C12_DUMMY,   15, ANY,  12, ANY, ANY,   1, WO | ARM_CP_NOP)

    // crn == 15
    ARM32_CP_REG_DEFINE(ZERO,             15, ANY,  15,   0, ANY,   1, RW | CONST(0))
    ARM32_CP_REG_DEFINE(TICONFIG,         15, ANY,  15,   1, ANY,   1, RW, RW_FNS(c15_ticonfig))   // Set TI925T configuration
    ARM32_CP_REG_DEFINE(C15_I_MAX,        15, ANY,  15,   2, ANY,   1, RW, FIELD(cp15.c15_i_max))  // Set I_max
    ARM32_CP_REG_DEFINE(C15_I_MIN,        15, ANY,  15,   3, ANY,   1, RW, FIELD(cp15.c15_i_min))  // Set I_min
    ARM32_CP_REG_DEFINE(THREADID,         15, ANY,  15,   4, ANY,   1, RW, RW_FNS(c15_threadid))   // Set thread-ID
    ARM32_CP_REG_DEFINE(TI925T_status,    15, ANY,  15,   8, ANY,   1, RW | ARM_CP_WFI | CONST(0)) // TI925T_status on Read or Wait-for-interrupt (deprecated) on Write

    /* TODO: Peripheral port remap register:
     * On OMAP2 mcr p15, 0, rn, c15, c2, 4 sets up the interrupt
     * controller base address at $rn & ~0xfff and map size of
     * 0x200 << ($rn & 0xfff), when MMU is off.  */
};

static ARMCPRegInfo strongarm_registers[] = {
    ARM32_CP_REG_DEFINE(STRONGARM_C9_DUMMY,    15,  10,   9,  10,  10,   1, RW | CONST(0))
};

static ARMCPRegInfo xscale_registers[] = {
    ARM32_CP_REG_DEFINE(XSCALE_C0_DUMMY,  15,  10,   0,  10,  10,   1, WO | ARM_CP_NOP)                // In OMAP or XSCALE writes to crn0 have no effect, but don't raise exception either
    ARM32_CP_REG_DEFINE(ACTLR,            15,   0,   1,   0,   1,   1, RW, FIELD(cp15.c1_xscaleauxcr)) // Auxiliary Control Register (Impl. defined)

    ARM32_CP_REG_DEFINE(CPAR,             15, ANY,  15,   1,   0,   1, RW | ARM_CP_FORCE_TB_END, RW_FNS(c15_cpar))
};

static ARMCPRegInfo feature_auxcr_registers[] = {
    // TODO: Should this be ARM_CP_IO?
    ARM32_CP_REG_DEFINE(ACTLR,            15,   0,   1,   0,   1,   1, RW | ARM_CP_IO, RW_FNS(c1_actlr)) // Auxiliary Control Register (Impl. defined)
};

static ARMCPRegInfo cpacr_register[] = {
    ARM32_CP_REG_DEFINE(CPACR,            15,   0,   1,   0,   2,   1, RW | ARM_CP_FORCE_TB_END, RW_FNS(c1_cpacr)) // Coprocessor Access Control Register
};

static ARMCPRegInfo feature_pmsa_registers[] = {
    // The params are:  name              cp, op1, crn, crm, op2,  el, extra_type, ...
    ARM32_CP_REG_DEFINE(MPUIR,            15,   0,   0,   0,   4,   1, RO, READFN(c0_mpuir))    // MPUIR, MPU Type Register

    ARM32_CP_REG_DEFINE(DFAR,             15,   0,   6,   0,   0,   1, RW, FIELD(cp15.c6_data)) // DFAR, Data Fault Address Register
    ARM32_CP_REG_DEFINE(IFAR,             15,   0,   6,   0,   2,   1, RW, FIELD(cp15.c6_insn)) // IFAR, Instruction Fault Address Register
    ARM32_CP_REG_DEFINE(DRBAR,            15,   0,   6,   1,   0,   1, RW, RW_FNS(c6_drbar))    // DRBAR, Data Region Base Address Register
    ARM32_CP_REG_DEFINE(DRSR,             15,   0,   6,   1,   2,   1, RW, RW_FNS(c6_drsr))     // DRSR, Data Region Size and Enable Register
    ARM32_CP_REG_DEFINE(DRACR,            15,   0,   6,   1,   4,   1, RW, RW_FNS(c6_dracr))    // DRACR, Data Region Access Control Register

    ARM32_CP_REG_DEFINE(RGNR,             15,   0,   6,   2,   0,   1, RW, RW_FNS(c6_rgnr))     // RGNR, MPU Region Number Register
};

// Based on https://developer.arm.com/documentation/ddi0460/d/System-Control/Register-descriptions/Register-allocation
static ARMCPRegInfo cortex_r5_registers[] = {
    // The params are:  name              cp, op1, crn, crm, op2,  el, extra_type, ...
    ARM32_CP_REG_DEFINE(SACTLR,           15,   0,  15,   0,   0,   1, RW)  // Secondary Auxiliary Control Register

    // Peripheral interface region registers, names are based on their related signals:
    // https://developer.arm.com/documentation/ddi0460/d/Signal-Descriptions/Configuration-signals
    ARM32_CP_REG_DEFINE(PPXR,             15,   0,  15,   0,   1,   1, RW, FIELD(cp15.c15_axi_region))          // LLPP Normal AXI Peripheral Interface Region Register
    ARM32_CP_REG_DEFINE(PPVR,             15,   0,  15,   0,   2,   1, RW, FIELD(cp15.c15_virtual_axi_region))  // LLPP Virtual AXI Peripheral Interface Region Register
    ARM32_CP_REG_DEFINE(PPHR,             15,   0,  15,   0,   3,   1, RW, FIELD(cp15.c15_ahb_region))          // AHB Peripheral Interface Region Register

    // Validation registers, names are based on their related signals:
    // https://developer.arm.com/documentation/ddi0460/d/Signal-Descriptions/Validation-signals
    ARM32_CP_REG_DEFINE(NVALIRQENSETR,    15,   0,  15,   1,   0,   1, RW)  // nVAL IRQ Enable Set Register
    ARM32_CP_REG_DEFINE(NVALFIQENSETR,    15,   0,  15,   1,   1,   1, RW)  // nVAL FIQ Enable Set Register
    ARM32_CP_REG_DEFINE(NVALRESETENSETR,  15,   0,  15,   1,   2,   1, RW)  // nVAL Reset Enable Set Register
    ARM32_CP_REG_DEFINE(VALEDBGRQENSETR,  15,   0,  15,   1,   3,   1, RW)  // VAL Debug Request Enable Set Register
    ARM32_CP_REG_DEFINE(NVALIRQENCLRR,    15,   0,  15,   1,   4,   1, RW)  // nVAL IRQ Enable Clear Register
    ARM32_CP_REG_DEFINE(NVALFIQENCLRR,    15,   0,  15,   1,   5,   1, RW)  // nVAL FIQ Enable Clear Register
    ARM32_CP_REG_DEFINE(NVALRESETENCLRR,  15,   0,  15,   1,   6,   1, RW)  // nVAL Reset Enable Clear Register
    ARM32_CP_REG_DEFINE(VALEDBGRQENCLRR,  15,   0,  15,   1,   7,   1, RW)  // VAL Debug Request Enable Clear Register

    ARM32_CP_REG_DEFINE(BOR1,             15,   0,  15,   2,   0,   1, RO)  // Build Options Register 1
    ARM32_CP_REG_DEFINE(BOR2,             15,   0,  15,   2,   1,   1, RO)  // Build Options Register 2
    ARM32_CP_REG_DEFINE(POR,              15,   0,  15,   2,   7,   1, RO)  // Pin Options Register

    ARM32_CP_REG_DEFINE(CFLR,             15,   0,  15,   3,   0,   1, RW)  // Correctable Fault Location Register
    ARM32_CP_REG_DEFINE(DCIALLR,          15,   0,  15,   5,   0,   1, WO)  // Data Cache Invalidate all Register
    ARM32_CP_REG_DEFINE(CSOR,             15,   0,  15,  14,   0,   1, WO)  // Cache Size Override Register
};

// Based on https://developer.arm.com/documentation/100400/0002/system-control/register-summary/implementation-defined-registers
static ARMCPRegInfo cortex_r8_registers[] = {
    // The params are:  name              cp, op1, crn, crm, op2,  el, extra_type, ...
    ARM32_CP_REG_DEFINE(PCR,              15,   0,  15,   0,   0,   0, RW)  // Power Control Register

    ARM32_CP_REG_DEFINE(CTDOR,            15,   0,  15,   1,   0,   1, WO)  // Cache and TCM Debug Operation Register
    ARM32_CP_REG_DEFINE(RADRLO,           15,   0,  15,   1,   1,   1, RW)  // RAM Access Data Register, bits[31:0]
    ARM32_CP_REG_DEFINE(RADRHI,           15,   0,  15,   1,   2,   1, RW)  // RAM Access Data Register, bits[63:32]

    // The below should only be available if ECC is implemented.
    ARM32_CP_REG_DEFINE(RAECCR,           15,   0,  15,   1,   3,   1, RW)  // RAM Access ECC Register

    ARM32_CP_REG_DEFINE(DEER0,            15,   0,  15,   2,   0,   1, RW, FIELD(cp15.c15_decc_entries[0]))  // D_ECC_ENTRY_0, Data ECC entry no. 0
    ARM32_CP_REG_DEFINE(DEER1,            15,   0,  15,   2,   1,   1, RW, FIELD(cp15.c15_decc_entries[1]))  // D_ECC_ENTRY_1, Data ECC entry no. 1
    ARM32_CP_REG_DEFINE(DEER2,            15,   0,  15,   2,   2,   1, RW, FIELD(cp15.c15_decc_entries[2]))  // D_ECC_ENTRY_2, Data ECC entry no. 2

    ARM32_CP_REG_DEFINE(IEER0,            15,   0,  15,   3,   0,   1, RW, FIELD(cp15.c15_iecc_entries[0]))  // I_ECC_ENTRY_0, Insn ECC entry no. 0
    ARM32_CP_REG_DEFINE(IEER1,            15,   0,  15,   3,   1,   1, RW, FIELD(cp15.c15_iecc_entries[1]))  // I_ECC_ENTRY_1, Insn ECC entry no. 1
    ARM32_CP_REG_DEFINE(IEER2,            15,   0,  15,   3,   2,   1, RW, FIELD(cp15.c15_iecc_entries[2]))  // I_ECC_ENTRY_2, Insn ECC entry no. 2

    // The below should only be accessible if ECC and TCM are implemented.
    ARM32_CP_REG_DEFINE(DTCMEER,          15,   0,  15,   4,   0,   1, RW, FIELD(cp15.c15_tcm_ecc_entries[TCM_CP15_OP2_DATA]))                    // DTCM_ECC_ENTRY, Data TCM ECC entry
    ARM32_CP_REG_DEFINE(ITCMEER,          15,   0,  15,   5,   0,   1, RW, FIELD(cp15.c15_tcm_ecc_entries[TCM_CP15_OP2_INSTRUCTION_OR_UNIFIED]))  // ITCM_ECC_ENTRY, Insn TCM ECC entry

    // CBAR is added with FEATURE_CBAR_RO.
};

static ARMCPRegInfo feature_generic_timer_registers[] = {
    ARM32_CP_REG_DEFINE(GENERIC_TIMER,    15, ANY,  14, ANY, ANY,   1, RW | ARM_CP_IO, RW_FNS(read_write_cp15))     // Generic Timer
};

static ARMCPRegInfo feature_thumb2ee_registers[] = {
    // ================== Coprocessor 14 ==================
    // The params are:  name              cp, op1, crn, crm, op2,  el,  extra_type, ...
    ARM32_CP_REG_DEFINE(TEECR,            14,   6,   0,   0,   0,   1,  RW | ARM_CP_FORCE_TB_END, RW_FNS(cp14_c6_teecr)) // Thumb EE Configuration Register
    ARM32_CP_REG_DEFINE(TEEHBR,           14,   6,   1,   0,   0,   1,  RW, FIELD(teehbr))         // Thumb EE Handler Base Register
};

// See ARMv7A/R manual's full list of CP15 registers for VMSA (B3.17.2) for the ones marked as "Added as part of the Multiprocessing Extensions.".
static ARMCPRegInfo feature_v7mp_mmu_registers[] = {
    // The params are:  name              cp, op1, crn, crm, op2,  el,  extra_type, ...
    ARM32_CP_REG_DEFINE(TLBIALLIS,        15,   0,   8,   3,   0,   1,  WO, WRITEFN(invalidate_all))            // Invalidate entire unified TLB Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIMVAIS,        15,   0,   8,   3,   1,   1,  WO, WRITEFN(invalidate_single))         // Invalidate unified TLB entry by MVA and ASID, Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIASIDIS,       15,   0,   8,   3,   2,   1,  WO, WRITEFN(invalidate_on_asid))        // Invalidate unified TLB by ASID match Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIMVAAIS,       15,   0,   8,   3,   3,   1,  WO, WRITEFN(invalidate_single_on_mva))  // Invalidate unified TLB entry by MVA all ASID Inner Shareable

    ARM32_CP_REG_DEFINE(TLBIMVAA,         15,   0,   8,   7,   3,   1,  WO, WRITEFN(invalidate_single_on_mva))  // Invalidate unified TLB entries by MVA all ASID
};

// See ARMv7A/R manual's full lists of CP15 registers for PMSA (B5.8.2) and VMSA (B3.17.2) CP15 registers for the ones marked as "Added as part of the Multiprocessing Extensions.".
static ARMCPRegInfo feature_v7mp_registers[] = {
    // The params are:  name              cp, op1, crn, crm, op2,  el,  extra_type, ...
    // This is not a barrier per se but it'll be treated like ISB cause we want to trigger TB invalidations.
    ARM32_CP_REG_DEFINE(ICIALLUIS,        15,   0,   7,   1,   0,   1,  WO | ARM_CP_BARRIER)   // Instruction Cache Invalidate All to PoU, Inner Shareable
    ARM32_CP_REG_DEFINE(BPIALLIS,         15,   0,   7,   1,   6,   1,  WO | IGNORED)          // Branch Predictor Invalidate All, Inner Shareable
};

static ARMCPRegInfo feature_v7sec_registers[] = {
    // The params are:  name              cp, op1, crn, crm, op2,  el,  extra_type, ...
    ARM32_CP_REG_DEFINE(VBAR,             15,   0,   12,   0,   0,  1,  RW, RW_FNS(c12_vbar)) // VBAR, Vector Base Address Register
};

static ARMCPRegInfo feature_cbar_ro[] = {
    // The params are:  name              cp, op1, crn, crm, op2,  el,  extra_type, ...
    ARM32_CP_REG_DEFINE(CBAR,             15,   4,   15,   0,   0,  1,  RO, FIELD(cp15.c15_cbar)) // CBAR, Configuration Base Address Register
};
// clang-format on

//  The keys are dynamically allocated so let's make TTable free them when removing the entry.
static void entry_remove_callback(TTable_entry *entry)
{
    tlib_free(entry->key);
    if(((ARMCPRegInfo *)entry->value)->dynamic) {
        tlib_free(entry->value);
    }
}

void cp_reg_add(CPUState *env, ARMCPRegInfo *reg_info)
{
    const bool ns = true;  //  TODO: Handle secure state banking in a correct way, when we add Secure Mode to this lib
    const bool is64 = reg_info->type & ARM_CP_64BIT;

    assert(reg_info->crn != ANY);

    //  Replicate the same register across many coproc addresses
    const int op1_start = reg_info->op1 < ANY ? reg_info->op1 : 0;
    const int op1_end = reg_info->op1 < ANY ? reg_info->op1 : 0x7;

    const int op2_start = reg_info->op2 < ANY ? reg_info->op2 : 0;
    const int op2_end = reg_info->op2 < ANY ? reg_info->op2 : 0x7;

    const int crm_start = reg_info->crm < ANY ? reg_info->crm : 0;
    const int crm_end = reg_info->crm < ANY ? reg_info->crm : 0xF;

    for(int op1 = op1_start; op1 <= op1_end; ++op1) {
        for(int op2 = op2_start; op2 <= op2_end; ++op2) {
            for(int crm = crm_start; crm <= crm_end; ++crm) {

                uint32_t *key = tlib_malloc(sizeof(uint32_t));
                *key = ENCODE_CP_REG(reg_info->cp, is64, ns, reg_info->crn, crm, op1, op2);

                if(reg_info->op1 == ANY || reg_info->op2 == ANY || reg_info->crm == ANY) {
                    ARMCPRegInfo *val = tlib_malloc(sizeof(*reg_info));
                    memcpy(val, reg_info, sizeof(*reg_info));

                    val->op1 = op1;
                    val->op2 = op2;
                    val->crm = crm;

                    val->dynamic = true;

                    cp_reg_add_with_key(env, env->cp_regs, key, val);
                } else {
                    cp_reg_add_with_key(env, env->cp_regs, key, reg_info);
                }
            }
        }
    }
}

void system_instructions_and_registers_reset(CPUState *env)
{
    TTable *cp_regs = env->cp_regs;

    int i;
    for(i = 0; i < cp_regs->count; i++) {
        ARMCPRegInfo *ri = cp_regs->entries[i].value;

        //  Nothing to be done for these because:
        //  * all the backing fields except the 'arm_core_config' ones are always reset to zero,
        //  * CONSTs have no backing fields and 'resetvalue' is always used when they're read.
        if((ri->resetvalue == 0) || (ri->type & ARM_CP_CONST)) {
            continue;
        }

        uint32_t width = (ri->type & ARM_CP_64BIT) ? 64 : 32;
        uint64_t value = width == 64 ? ri->resetvalue : ri->resetvalue & UINT32_MAX;

        tlib_printf(LOG_LEVEL_NOISY, "Resetting value for '%s': 0x%" PRIx64, ri->name, value);
        if(ri->fieldoffset) {
            memcpy((void *)env + ri->fieldoffset, &value, (size_t)(width / 8));
        } else if(ri->writefn) {
            ri->writefn(env, ri, value);
        } else {
            //  Shouldn't happen so let's make sure it doesn't.
            tlib_assert_not_reached();
        }
    }
}

static int count_cp_array(const ARMCPRegInfo *array, const int items)
{
    int i = items, num = 0;
    while(i--) {
        int many = 1;
        many *= array[i].crm == ANY ? 16 : 1;
        many *= array[i].op1 == ANY ? 8 : 1;
        many *= array[i].op2 == ANY ? 8 : 1;

        num += many;
    }

    return num;
}

#define ARM_CP_ARRAY_COUNT_ANY(array) count_cp_array(array, ARM_CP_ARRAY_COUNT(array))

//  Calculate ttable size
inline static int count_extra_registers(const CPUState *env)
{
    //  c13 registers replaced by dummy counterparts
    assert(ARM_CP_ARRAY_COUNT_ANY(has_cp15_c13_dummy_registers) == ARM_CP_ARRAY_COUNT_ANY(has_cp15_c13_registers));

    int extra_regs = 0;
    if(arm_feature(env, ARM_FEATURE_OMAPCP)) {
        //  Seed dummy r/w NOP register on c0
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(omap_registers);
    }
    if(arm_feature(env, ARM_FEATURE_XSCALE)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(xscale_registers);

        //  XSCALE-specific handling of sctlr
        sctlr_register[0].type &= ~ARM_CP_SUPPRESS_TB_END;
        sctlr_register[0].crm = ANY;
    }
    if(arm_feature(env, ARM_FEATURE_STRONGARM)) {
        //  Seed dummy r/w NOP register on c0
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(strongarm_registers);
    }

    if(arm_feature(env, ARM_FEATURE_V7) || ARM_CPUID(env) == ARM_CPUID_ARM11MPCORE) {
        /* The MPIDR was standardised in v7; prior to
         * this it was implemented only in the 11MPCore.
         * For all other pre-v7 cores it does not exist.
         */
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(mpidr_register);
    }

    if(!arm_feature(env, ARM_FEATURE_V6)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_pre_v6_registers);
    }
    if(arm_feature(env, ARM_FEATURE_V6)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_v6_registers);
    }
    if(!arm_feature(env, ARM_FEATURE_V7)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_pre_v7_registers);
    }
    if(arm_feature(env, ARM_FEATURE_V7)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_v7_registers);
    }

    if(arm_feature(env, ARM_FEATURE_MPU)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_mpu_registers);
    } else {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(has_mmu_registers);
    }

    if(arm_feature(env, ARM_FEATURE_PMSA)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_pmsa_registers);
    }

    if(!arm_feature(env, ARM_FEATURE_XSCALE)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(cpacr_register);
    }

    if(arm_feature(env, ARM_FEATURE_AUXCR)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_auxcr_registers);
    }

    if(arm_feature(env, ARM_FEATURE_MPU) && !arm_feature(env, ARM_FEATURE_PMSA)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(has_mpu_fault_addr_register);
    }

    if(arm_feature(env, ARM_FEATURE_GENERIC_TIMER)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_generic_timer_registers);
    }

    if(arm_feature(env, ARM_FEATURE_THUMB2EE)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_thumb2ee_registers);
    }

    if(arm_feature(env, ARM_FEATURE_V7MP)) {
        if(!arm_feature(env, ARM_FEATURE_MPU)) {
            extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_v7mp_mmu_registers);
        }
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_v7mp_registers);
    }

    if(arm_feature(env, ARM_FEATURE_V7SEC)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_v7sec_registers);
    }

    if(arm_feature(env, ARM_FEATURE_CBAR_RO)) {
        extra_regs += ARM_CP_ARRAY_COUNT_ANY(feature_cbar_ro);
    }

    switch(env->cp15.c0_cpuid) {
        case ARM_CPUID_CORTEXR5:
        case ARM_CPUID_CORTEXR5F:
            extra_regs += ARM_CP_ARRAY_COUNT_ANY(cortex_r5_registers);
            break;
        case ARM_CPUID_CORTEXR8:
            extra_regs += ARM_CP_ARRAY_COUNT_ANY(cortex_r8_registers);
            break;
    }

    extra_regs += ARM_CP_ARRAY_COUNT_ANY(has_cp15_c13_registers);
    extra_regs += ARM_CP_ARRAY_COUNT_ANY(sctlr_register);

    return extra_regs;
}

#define regs_array_add(env, regs) cp_regs_add(env, regs, ARM_CP_ARRAY_COUNT(regs));

//  Seed ttable with registers
inline static void populate_ttable(CPUState *env)
{
    //  Generate dummy registers
    if(arm_feature(env, ARM_FEATURE_OMAPCP)) {
        //  Seed dummy r/w NOP register on c0
        regs_array_add(env, omap_registers);
    }
    if(arm_feature(env, ARM_FEATURE_XSCALE)) {
        regs_array_add(env, xscale_registers);
    }
    if(arm_feature(env, ARM_FEATURE_STRONGARM)) {
        regs_array_add(env, strongarm_registers);
    }

    regs_array_add(env, general_coprocessor_registers);

    if(arm_feature(env, ARM_FEATURE_V7) || ARM_CPUID(env) == ARM_CPUID_ARM11MPCORE) {
        regs_array_add(env, mpidr_register);
    }

    if(!arm_feature(env, ARM_FEATURE_V6)) {
        regs_array_add(env, feature_pre_v6_registers);
    }
    if(arm_feature(env, ARM_FEATURE_V6)) {
        regs_array_add(env, feature_v6_registers);
    }
    if(!arm_feature(env, ARM_FEATURE_V7)) {
        regs_array_add(env, feature_pre_v7_registers);
    }
    if(arm_feature(env, ARM_FEATURE_V7)) {
        regs_array_add(env, feature_v7_registers);
    }

    if(arm_feature(env, ARM_FEATURE_MPU)) {
        regs_array_add(env, feature_mpu_registers);
    } else {
        regs_array_add(env, has_mmu_registers);
    }

    if(arm_feature(env, ARM_FEATURE_PMSA)) {
        regs_array_add(env, feature_pmsa_registers);
    }

    if(!arm_feature(env, ARM_FEATURE_XSCALE)) {
        regs_array_add(env, cpacr_register);
    }

    if(arm_feature(env, ARM_FEATURE_AUXCR)) {
        regs_array_add(env, feature_auxcr_registers);
    }

    if(arm_feature(env, ARM_FEATURE_MPU) && !arm_feature(env, ARM_FEATURE_PMSA)) {
        regs_array_add(env, has_mpu_fault_addr_register);
    }

    if(arm_feature(env, ARM_FEATURE_GENERIC_TIMER)) {
        regs_array_add(env, feature_generic_timer_registers);
    }

    if(arm_feature(env, ARM_FEATURE_THUMB2EE)) {
        regs_array_add(env, feature_thumb2ee_registers);
    }

    if(arm_feature(env, ARM_FEATURE_V7MP)) {
        if(!arm_feature(env, ARM_FEATURE_MPU)) {
            regs_array_add(env, feature_v7mp_mmu_registers);
        }
        regs_array_add(env, feature_v7mp_registers);
    }

    if(arm_feature(env, ARM_FEATURE_V7SEC)) {
        regs_array_add(env, feature_v7sec_registers);
    }

    if(arm_feature(env, ARM_FEATURE_CBAR_RO)) {
        regs_array_add(env, feature_cbar_ro);
    }

    switch(env->cp15.c0_cpuid) {
        case ARM_CPUID_CORTEXR5:
        case ARM_CPUID_CORTEXR5F:
            regs_array_add(env, cortex_r5_registers);
            break;
        case ARM_CPUID_CORTEXR8:
            regs_array_add(env, cortex_r8_registers);
            break;
    }

    //  c13 are always present, but without ARM_FEATURE_V6K should be read as 0
    if(arm_feature(env, ARM_FEATURE_V6K) || arm_feature(env, ARM_FEATURE_V7)) {
        regs_array_add(env, has_cp15_c13_registers);
    } else {
        regs_array_add(env, has_cp15_c13_dummy_registers);
    }
    regs_array_add(env, sctlr_register);
}

void system_instructions_and_registers_init(CPUState *env)
{
    //  This would break logic handling ACTLR (Auxiliary Control Register) - we assume that these are mutually exclusive
    assert(!(arm_feature(env, ARM_FEATURE_XSCALE) && arm_feature(env, ARM_FEATURE_AUXCR)));
    //  In v7, Security Extensions should not be enabled when PMSA is enabled
    assert(!(arm_feature(env, ARM_FEATURE_V7SEC) && arm_feature(env, ARM_FEATURE_PMSA)));

    int extra_regs = count_extra_registers(env);

    //  Create ttable
    uint32_t ttable_size = ARM_CP_ARRAY_COUNT_ANY(general_coprocessor_registers) + extra_regs;
    env->cp_regs = ttable_create(ttable_size, entry_remove_callback, ttable_compare_key_uint32);

    populate_ttable(env);
    ttable_sort_by_keys(env->cp_regs);
}
