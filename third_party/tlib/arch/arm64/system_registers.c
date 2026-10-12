/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#include "bit_helper.h"
#include "cpu.h"
#include "cpu_names.h"
#include "global_helper.h"
#include "system_registers.h"
#include "ttable.h"

/* This file is named 'system_registers.h' but it also contains definitions of the system instructions.
 *
 * Beware the 'register name' vs 'instruction mnemonic' ambiguity because even ARMCPRegInfo.name is in fact an instruction
 * mnemonic not a register name.
 *
 * For example, an 'MRS ELR_EL1' instruction is a read with a ELR_EL1 mnemonic but it doesn't always read the ELR_EL1 register. In
 * certain situations, i.e., if EL == 2 and HCR_EL2.E2H is set, it should return value of the ELR_EL2 register.
 *
 * Basically all the mnemonics used in MRS/MSR (AArch64), MRC/MCR (AArch32), AT, DC, IC, TLBI etc. instructions should have their
 * entry in 'cp_regs'.
 */

inline const char *sysreg_patch_lookup_name(const char *name)
{
    const char *lookup_name = name;

    if(strcasecmp(name, "DBGDTRRX_EL0") == 0 || strcasecmp(name, "DBGDTRTX_EL0") == 0) {
        lookup_name = "DBGDTR_RX_TX_EL0";
    } else if(strcasecmp(lookup_name, "ICV_AP0R_0") == 0) {
        lookup_name = "ICC_AP0R_0";
    } else if(strcasecmp(lookup_name, "ICV_AP0R_1") == 0) {
        lookup_name = "ICC_AP0R_1";
    } else if(strcasecmp(lookup_name, "ICV_AP0R_2") == 0) {
        lookup_name = "ICC_AP0R_2";
    } else if(strcasecmp(lookup_name, "ICV_AP0R_3") == 0) {
        lookup_name = "ICC_AP0R_3";
    } else if(strcasecmp(lookup_name, "ICV_AP1R_0") == 0) {
        lookup_name = "ICC_AP1R_0";
    } else if(strcasecmp(lookup_name, "ICV_AP1R_1") == 0) {
        lookup_name = "ICC_AP1R_1";
    } else if(strcasecmp(lookup_name, "ICV_AP1R_2") == 0) {
        lookup_name = "ICC_AP1R_2";
    } else if(strcasecmp(lookup_name, "ICV_AP1R_3") == 0) {
        lookup_name = "ICC_AP1R_3";
    } else if(strcasecmp(lookup_name, "ICV_BPR0") == 0) {
        lookup_name = "ICC_BPR0";
    } else if(strcasecmp(lookup_name, "ICV_BPR1") == 0) {
        lookup_name = "ICC_BPR1";
    } else if(strcasecmp(lookup_name, "ICV_CTLR") == 0) {
        lookup_name = "ICC_CTLR";
    } else if(strcasecmp(lookup_name, "ICV_DIR") == 0) {
        lookup_name = "ICC_DIR";
    } else if(strcasecmp(lookup_name, "ICV_EOIR0") == 0) {
        lookup_name = "ICC_EOIR0";
    } else if(strcasecmp(lookup_name, "ICV_EOIR1") == 0) {
        lookup_name = "ICC_EOIR1";
    } else if(strcasecmp(lookup_name, "ICV_HPPIR0") == 0) {
        lookup_name = "ICC_HPPIR0";
    } else if(strcasecmp(lookup_name, "ICV_HPPIR1") == 0) {
        lookup_name = "ICC_HPPIR1";
    } else if(strcasecmp(lookup_name, "ICV_IAR0") == 0) {
        lookup_name = "ICC_IAR0";
    } else if(strcasecmp(lookup_name, "ICV_IAR1") == 0) {
        lookup_name = "ICC_IAR1";
    } else if(strcasecmp(lookup_name, "ICV_IGRPEN0") == 0) {
        lookup_name = "ICC_IGRPEN0";
    } else if(strcasecmp(lookup_name, "ICV_IGRPEN1") == 0) {
        lookup_name = "ICC_IGRPEN1";
    } else if(strcasecmp(lookup_name, "ICV_PMR") == 0) {
        lookup_name = "ICC_PMR";
    } else if(strcasecmp(lookup_name, "ICV_RPR") == 0) {
        lookup_name = "ICC_RPR";
    }

    return lookup_name;
}

/* Helpers for mnemonics with a complex mnemonic->register translation. */

static inline uint64_t *cpacr_el1_register_pointer(CPUState *env)
{
    return el2_and_hcr_el2_e2h_set(env) ? &env->cp15.cptr_el[2] : &env->cp15.cpacr_el1;
}

static inline uint64_t mpidr_el1_register(CPUState *env)
{
    uint32_t affinity = tlib_get_mp_index();

    //  Aff3 is not located immediately after Aff2 in MPIDR
    uint64_t aff3 = extract64(affinity, 24, 8) << 32;
    uint32_t aff0_1_2 = extract32(affinity, 0, 24);

    if(arm_current_el(env) == 1 && arm_is_el2_enabled(env)) {
        return env->cp15.vmpidr_el2 | aff0_1_2 | aff3;
    }
    return env->arm_core_config.mpidr | aff0_1_2 | aff3;
}

static inline uint64_t *spsr_el1_register_pointer(CPUState *env)
{
    uint32_t spsr_idx = el2_and_hcr_el2_e2h_set(env) ? SPSR_EL2 : SPSR_EL1;
    return &env->banked_spsr[spsr_idx];
}

/* Other helpers */

static inline uint64_t get_id_aa64pfr0_value(CPUState *env)
{
    uint64_t return_value = env->arm_core_config.isar.id_aa64pfr0;

    if(!arm_feature(env, ARM_FEATURE_EL3)) {
        return_value = FIELD_DP64(return_value, ID_AA64PFR0, EL3, 0);
    }

    if(!arm_feature(env, ARM_FEATURE_EL2)) {
        return_value = FIELD_DP64(return_value, ID_AA64PFR0, EL2, 0);
    }

    //  FP16 isn't currently supported so let's override FP field.
    //  FP and AdvSIMD fields have to be equal according to the manual.
    if(FIELD_EX64(return_value, ID_AA64PFR0, FP) == 1) {
        return_value = FIELD_DP64(return_value, ID_AA64PFR0, ADVSIMD, 0);
        return_value = FIELD_DP64(return_value, ID_AA64PFR0, FP, 0);
    }

    return return_value;
}

enum pmsav8_register_type {
    BASE_ADDRESS,
    HYPER_BASE_ADDRESS,
    LIMIT_ADDRESS,
    HYPER_LIMIT_ADDRESS,
};

static inline uint64_t pmsav8_mark_overlapping_regions(CPUState *env, int base_region_index, uint32_t address_start,
                                                       uint32_t address_end, bool is_hyper)
{
    pmsav8_region *regions;
    int regions_count;
    uint32_t base_region_mask = 1 << base_region_index;
    uint64_t overlapping_mask = 0;

    if(is_hyper) {
        regions = env->pmsav8.hregions;
        regions_count = pmsav8_number_of_el2_regions(env);
    } else {
        regions = env->pmsav8.regions;
        regions_count = pmsav8_number_of_el1_regions(env);
    }

    for(int index = 0; index < regions_count; index++) {
        if(!regions[index].enabled || index == base_region_index) {
            continue;
        }

        uint32_t i_address_start = regions[index].address_start;
        uint32_t i_address_end = regions[index].address_limit;

        //  The first two check if the 'regions[index]' region starts or ends in the region being added.
        //  The third one checks the only remaining overlapping option: whether the region being added
        //  is completely contained within the 'regions[index]' region.
        if(((i_address_start >= address_start) && (i_address_start <= address_end)) ||
           ((i_address_end >= address_start) && (i_address_end <= address_end)) ||
           ((address_start > i_address_start) && (address_start < i_address_end))) {
            regions[index].overlapping_regions_mask |= base_region_mask;
            overlapping_mask |= (1 << index);
        }
    }
    return overlapping_mask;
}

static inline void pmsav8_unmark_overlapping_regions(CPUARMState *env, pmsav8_region *regions, uint64_t base_region_mask,
                                                     uint64_t mask)
{
    int index = 0;
    while(mask != 0) {
        if(mask & 0b1) {
            regions[index].overlapping_regions_mask ^= base_region_mask;
        }
        mask = mask << 1;
        index++;
    }
}

static inline void hyp_pmsav8_regions_bitmask_enable(CPUState *env, uint32_t bitmask)
{
    for(int i = 0; i < MAX_MPU_REGIONS; i += 1, bitmask >>= 1) {
        env->pmsav8.hregions[i].enabled = bitmask & 1;
    }
}

static inline uint32_t hyp_pmsav8_regions_bitmask_current(CPUState *env)
{
    uint32_t result = 0;

    for(int i = MAX_MPU_REGIONS - 1; i >= 0; i -= 1) {
        result = (result << 1) | (env->pmsav8.hregions[i].enabled ? 1 : 0);
    }

    return result;
}

static inline void set_pmsav8_region(CPUState *env, enum pmsav8_register_type type, unsigned int region_index, uint32_t value)
{
    if(region_index >= MAX_MPU_REGIONS) {
        //  Writing a value outside of the valid range is UNPREDICTABLE.
        //  We opt for just ignoring it
        return;
    }
    bool is_hyper = (type == HYPER_BASE_ADDRESS) || (type == HYPER_LIMIT_ADDRESS);
    pmsav8_region *regions = is_hyper ? &env->pmsav8.hregions[0] : &env->pmsav8.regions[0];
    pmsav8_region *region = &regions[region_index];

    switch(type) {
        case BASE_ADDRESS:
        case HYPER_BASE_ADDRESS:
            region->address_start = value & ~0x3Flu;
            region->execute_never = extract32(value, 0, 1);
            region->access_permission_bits = extract32(value, 1, 2);
            region->shareability_attribute = extract32(value, 3, 2);
            break;
        case LIMIT_ADDRESS:
        case HYPER_LIMIT_ADDRESS:
            region->enabled = extract32(value, 0, 1);
            region->mair_attribute = extract32(value, 1, 3);
            region->address_limit = value | 0x3Flu;
            break;
    }

    //  Need to unset the mask in other regions
    pmsav8_unmark_overlapping_regions(env, regions, 1 << region_index, region->overlapping_regions_mask);
    //  And mark the ones that now overlap
    region->overlapping_regions_mask =
        pmsav8_mark_overlapping_regions(env, region_index, region->address_start, region->address_limit, is_hyper);

    tlb_flush(env, 1, true);
}

static inline uint32_t get_pmsav8_region(CPUState *env, enum pmsav8_register_type type, unsigned int region_index)
{
    if(region_index >= MAX_MPU_REGIONS) {
        //  Reading a value outside of the valid range is UNPREDICTABLE.
        //  We opt for just returning 0
        return 0;
    }
    bool is_hyper = (type == HYPER_BASE_ADDRESS) || (type == HYPER_LIMIT_ADDRESS);
    pmsav8_region *region = is_hyper ? &env->pmsav8.hregions[region_index] : &env->pmsav8.regions[region_index];

    uint32_t return_value;
    switch(type) {
        case BASE_ADDRESS:
        case HYPER_BASE_ADDRESS:
            return_value = region->address_start;
            return_value = deposit32(return_value, 0, 1, region->execute_never);
            return_value = deposit32(return_value, 1, 2, region->access_permission_bits);
            return_value = deposit32(return_value, 3, 2, region->shareability_attribute);
            break;
        case LIMIT_ADDRESS:
        case HYPER_LIMIT_ADDRESS:
            return_value = region->address_limit & ~0x3Flu;
            return_value = deposit32(return_value, 0, 1, region->enabled);
            return_value = deposit32(return_value, 1, 3, region->mair_attribute);
            break;
    }
    return return_value;
}

//  Many 'MRS/MSR *_EL1' instructions access '*_EL2' registers if EL is 2 and HCR_EL2's E2H bit is set.
#define RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(width, mnemonic, field_base) \
    RW_FUNCTIONS_PTR(width, mnemonic, &field_base[el2_and_hcr_el2_e2h_set(env) ? 2 : 1])

static inline bool is_generic_timer_cntp_cntv_register(uint8_t op0, uint8_t op1, uint8_t crn, uint8_t crm, uint8_t op2)
{
    //  crm is 2 for CNTP_* and 3 for CNTV_* registers.
    return op0 == 3 && op1 == 3 && crn == 14 && (crm == 2 || crm == 3) && op2 <= 2;
}

static inline uint32_t encode_as_aarch64_register(uint8_t op0, uint8_t op1, uint8_t crn, uint8_t crm, uint8_t op2)
{
    return (op0 << CP_REG_ARM64_SYSREG_OP0_SHIFT) | (op1 << CP_REG_ARM64_SYSREG_OP1_SHIFT) |
           (crn << CP_REG_ARM64_SYSREG_CRN_SHIFT) | (crm << CP_REG_ARM64_SYSREG_CRM_SHIFT) |
           (op2 << CP_REG_ARM64_SYSREG_OP2_SHIFT);
}

static inline uint32_t encode_gic_register(CPUState *env, const ARMCPRegInfo *info)
{
    //  This function is used for accessing GIC registers
    tlib_assert(info->type & ARM_CP_GIC);
    //  There's no op0 in AArch32 instructions. It's the same for all GIC registers on AArch64.
    uint8_t op0 = !is_a64(env) ? 0x3 : info->op0;
    uint8_t op1 = info->op1;
    uint8_t crn = info->crn;
    uint8_t crm = info->crm;
    uint8_t op2 = info->op2;

    //  Encoding adjustments for GIC registers accessed from AArch32. It's very similar to AArch64.
    if(!is_a64(env)) {
        //  Adjustments are needed for 64-bit GIC registers accessed with AArch32 MCRR/MRRC instructions
        //  which are only identified by op1 and crm. On AArch32 all of them have crm=12. On AArch64 all of
        //  them have op0=3, op1=0, crn=12, crm=11. The remaining ones are:
        //  * ICC_ASGI1R (AArch32: op1=1; AArch64: op2=6)
        //  * ICC_SGI0R  (AArch32: op1=2; AArch64: op2=7)
        //  * ICC_SGI1R  (AArch32: op1=0; AArch64: op2=5)
        if(unlikely(info->type & ARM_CP_64BIT) && info->type & ARM_CP_GIC) {
            tlib_assert(crm == 12 && op1 <= 2);
            op1 = 0;
            crn = 12;
            crm = 11;
            op2 = 5 + info->op1;
        }
    }
    return encode_as_aarch64_register(op0, op1, crn, crm, op2);
}

static inline uint32_t encode_aarch64_generic_timer_register(CPUState *env, const ARMCPRegInfo *info)
{
    //  This function is used for accessing Generic Timer registers on AArch64
    tlib_assert(info->type & ARM_CP_64BIT && info->type & ARM_CP_GTIMER);
    //  There's no op0 in AArch32 instructions
    uint8_t op0 = !is_a64(env) ? 0x3 : info->op0;
    uint8_t op1 = info->op1;
    uint8_t crn = info->crn;
    uint8_t crm = info->crm;
    uint8_t op2 = info->op2;

    //  EL2 accesses to EL1 Physical/Virtual Timers (CNTP_*/CNTV_*) are redirected if HCR_EL2.E2H set.
    if(el2_and_hcr_el2_e2h_set(env) && is_generic_timer_cntp_cntv_register(op0, op1, crn, crm, op2)) {
        //  ARMv8-A manual's rule LLSLV: in secure state redirect to Secure EL2 Physical/Virtual Timer (CNTHPS_*/CNTHVS_*).
        //  The Secure EL2 timers are added by ARMv8.4's Secure EL2 extension. It's unclear what to do in secure state
        //  without the extension so let's just make sure the extension is disabled and state isn't secure.
        tlib_assert(!isar_feature_aa64_sel2(&env->arm_core_config.isar));
        tlib_assert(!arm_is_secure_below_el3(env));

        //  ARMv8-A manual's rule RZRWZ: in non-secure state redirect to Non-secure EL2 Physical/Virtual Timer (CNTHP_*/CNTHV_*).
        //  Equivalent CNTP_*->CNTHP_* and CNTV_*->CNTHV_* register opcodes only differ in op1 which is 4 instead of 3.
        op1 = 4;
    }
    return encode_as_aarch64_register(op0, op1, crn, crm, op2);
}

// clang-format off
READ_FUNCTION(64, mpidr_el1, mpidr_el1_register(env))

RW_FUNCTIONS(64, fpcr, vfp_get_fpcr(env), vfp_set_fpcr(env, value))
RW_FUNCTIONS(64, fpsr, vfp_get_fpsr(env), vfp_set_fpsr(env, value))

RW_FUNCTIONS(64, generic_timer_aarch64,
             tlib_read_system_register_generic_timer_64(encode_aarch64_generic_timer_register(env, info)),
             tlib_write_system_register_generic_timer_64(encode_aarch64_generic_timer_register(env, info), value))

RW_FUNCTIONS(64, generic_timer_aarch32_32, tlib_read_system_register_generic_timer_32(encode_as_aarch32_32bit_register(info)),
             tlib_write_system_register_generic_timer_32(encode_as_aarch32_32bit_register(info), value))

RW_FUNCTIONS(64, generic_timer_aarch32_64, tlib_read_system_register_generic_timer_64(encode_as_aarch32_64bit_register(info)),
             tlib_write_system_register_generic_timer_64(encode_as_aarch32_64bit_register(info), value))

//  Handles AArch32 and AArch64 GIC system registers
RW_FUNCTIONS(64, interrupt_cpu_interface, tlib_read_system_register_interrupt_cpu_interface(encode_gic_register(env, info)),
             tlib_write_system_register_interrupt_cpu_interface(encode_gic_register(env, info), value))

ACCESS_FUNCTION(interrupt_cpu_interface, env->arm_core_config.gic_cpu_interface_version == 0 ? CP_ACCESS_TRAP : CP_ACCESS_OK)

RW_FUNCTIONS(64, nzcv, nzcv_read(env), nzcv_write(env, value))

RW_FUNCTIONS_PTR(64, cpacr_el1, cpacr_el1_register_pointer(env))
RW_FUNCTIONS_PTR(64, spsr_el1, spsr_el1_register_pointer(env))

// TODO: For all of them their EL12 mnemonic should be undefined unless E2H is set.
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, contextidr_el1,   env->cp15.contextidr_el)
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, elr_el1,          env->elr_el)
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, esr_el1,          env->cp15.esr_el)
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, far_el1,          env->cp15.far_el)
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, mair_el1,         env->cp15.mair_el)
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, sctlr_el1,        env->cp15.sctlr_el)
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, scxtnum_el1,      env->scxtnum_el)
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, tcr_el1,          env->cp15.tcr_el)
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, tfsr_el1,         env->cp15.tfsr_el)
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, ttbr0_el1,        env->cp15.ttbr0_el)
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, ttbr1_el1,        env->cp15.ttbr1_el)
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, vbar_el1,         env->cp15.vbar_el)
RW_FUNCTIONS_EL1_ACCESSING_EL2_IF_E2H_SET(64, zcr_el1,          env->vfp.zcr_el)

WRITE_FUNCTION(64, ats1, {
    if(arm_feature(env, ARM_FEATURE_PMSA)) {
        //  TODO: Translate VA to IPA here
        env->cp15.par_ns = value & TARGET_PAGE_MASK;
    } else {
        tlib_printf(LOG_LEVEL_WARNING, "%s register is only implemented for CPUs with the PMSA feature", info->name);
    }
});

/* We want to keep everything except BASEADRESS and ENABLEELx */
#define IMP_XTCMREGIONR_MASK 0x1FFC

WRITE_FUNCTION(64, tcm_region, {
    uint32_t current_address = value >> 13;
    uint32_t el01_enabled = value & 0x1;
    uint32_t el2_enabled = value & 0x2;

    *sysreg_field_ptr(env, info) &= IMP_XTCMREGIONR_MASK;
    *sysreg_field_ptr(env, info) |= value & ~IMP_XTCMREGIONR_MASK;
    tlib_on_tcm_mapping_update(info->op2, current_address, el01_enabled, el2_enabled);

    tlb_flush(env, 1, true);
});

ACCESS_FUNCTION(rndr, isar_feature_aa64_rndr(&env->arm_core_config.isar) ? CP_ACCESS_OK : CP_ACCESS_TRAP);
READ_FUNCTION(64, rndr, (nzcv_write(env, 0), tlib_get_random_ulong()));

/* PMSAv8 accessors */
#define PRSELR_REGION_MASK 0xFF
#define RW_FUNCTIONS_PMSAv8(width)                                                                                            \
    READ_FUNCTION(width, prbar, get_pmsav8_region(env, BASE_ADDRESS, env->pmsav8.prselr &PRSELR_REGION_MASK))                 \
    READ_FUNCTION(width, prlar, get_pmsav8_region(env, LIMIT_ADDRESS, env->pmsav8.prselr &PRSELR_REGION_MASK))                \
    READ_FUNCTION(width, hprbar, get_pmsav8_region(env, HYPER_BASE_ADDRESS, env->pmsav8.hprselr &PRSELR_REGION_MASK))         \
    READ_FUNCTION(width, hprlar, get_pmsav8_region(env, HYPER_LIMIT_ADDRESS, env->pmsav8.hprselr &PRSELR_REGION_MASK))        \
    WRITE_FUNCTION(width, prbar, set_pmsav8_region(env, BASE_ADDRESS, env->pmsav8.prselr &PRSELR_REGION_MASK, value))         \
    WRITE_FUNCTION(width, prlar, set_pmsav8_region(env, LIMIT_ADDRESS, env->pmsav8.prselr &PRSELR_REGION_MASK, value))        \
    WRITE_FUNCTION(width, hprbar, set_pmsav8_region(env, HYPER_BASE_ADDRESS, env->pmsav8.hprselr &PRSELR_REGION_MASK, value)) \
    WRITE_FUNCTION(width, hprlar, set_pmsav8_region(env, HYPER_LIMIT_ADDRESS, env->pmsav8.hprselr &PRSELR_REGION_MASK, value))

#define RW_FUNCTIONS_PMSAv8_REGISTERS(width, index)                                                 \
    READ_FUNCTION(width, prbarn##index, get_pmsav8_region(env, BASE_ADDRESS, index))                \
    READ_FUNCTION(width, prlarn##index, get_pmsav8_region(env, LIMIT_ADDRESS, index))               \
    READ_FUNCTION(width, hprbarn##index, get_pmsav8_region(env, HYPER_BASE_ADDRESS, index))         \
    READ_FUNCTION(width, hprlarn##index, get_pmsav8_region(env, HYPER_LIMIT_ADDRESS, index))        \
    WRITE_FUNCTION(width, prbarn##index, set_pmsav8_region(env, BASE_ADDRESS, index, value))        \
    WRITE_FUNCTION(width, prlarn##index, set_pmsav8_region(env, LIMIT_ADDRESS, index, value))       \
    WRITE_FUNCTION(width, hprbarn##index, set_pmsav8_region(env, HYPER_BASE_ADDRESS, index, value)) \
    WRITE_FUNCTION(width, hprlarn##index, set_pmsav8_region(env, HYPER_LIMIT_ADDRESS, index, value))

RW_FUNCTIONS_PMSAv8_REGISTERS(64, 0)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 1)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 2)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 3)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 4)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 5)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 6)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 7)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 8)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 9)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 10)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 11)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 12)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 13)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 14)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 15)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 16)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 17)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 18)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 19)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 20)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 21)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 22)
RW_FUNCTIONS_PMSAv8_REGISTERS(64, 23)

RW_FUNCTIONS_PMSAv8(64)

RW_FUNCTIONS(64, hprenr, hyp_pmsav8_regions_bitmask_current(env), hyp_pmsav8_regions_bitmask_enable(env, value))

/* PSTATE accessors */

#define RW_PSTATE_FUNCTIONS(mnemonic, pstate_field) \
    RW_FUNCTIONS(64, mnemonic, pstate_read(env) & pstate_field, pstate_write_masked(env, value, pstate_field))

RW_PSTATE_FUNCTIONS(allint, PSTATE_ALLINT)
RW_PSTATE_FUNCTIONS(dit,    PSTATE_DIT)
RW_PSTATE_FUNCTIONS(pan,    PSTATE_PAN)
RW_PSTATE_FUNCTIONS(spsel,  PSTATE_SP)
RW_PSTATE_FUNCTIONS(ssbs,   PSTATE_SSBS)
RW_PSTATE_FUNCTIONS(tco,    PSTATE_TCO)
RW_PSTATE_FUNCTIONS(uao,    PSTATE_UAO)

/* 'arm_core_config'-reading functions */

#define READ_CONFIG(name, config_field_name) \
    READ_FUNCTION(64, name, env->arm_core_config.config_field_name)

READ_CONFIG(ccsidr_el1,        ccsidr[env->cp15.csselr_el[1]])
READ_CONFIG(ccsidr2_el1,       ccsidr[env->cp15.csselr_el[1]]   >>  32)
READ_CONFIG(clidr_el1,         clidr)
READ_CONFIG(ctr_el0,           ctr)
READ_CONFIG(dczid,             dcz_blocksize)
READ_CONFIG(id_aa64afr0_el1,   id_aa64afr0)
READ_CONFIG(id_aa64afr1_el1,   id_aa64afr1)
READ_CONFIG(id_aa64dfr0_el1,   isar.id_aa64dfr0)
READ_CONFIG(id_aa64isar0_el1,  isar.id_aa64isar0)
READ_CONFIG(id_aa64isar1_el1,  isar.id_aa64isar1)
READ_CONFIG(id_aa64mmfr0_el1,  isar.id_aa64mmfr0)
READ_CONFIG(id_aa64mmfr1_el1,  isar.id_aa64mmfr1)
READ_CONFIG(id_aa64mmfr2_el1,  isar.id_aa64mmfr2)
READ_FUNCTION(64, id_aa64pfr0_el1, get_id_aa64pfr0_value(env))
READ_CONFIG(id_aa64pfr1_el1,   isar.id_aa64pfr1)
READ_CONFIG(id_aa64smfr0_el1,  isar.id_aa64smfr0)
READ_CONFIG(id_aa64zfr0_el1,   isar.id_aa64zfr0)
READ_CONFIG(id_afr0,           id_afr0)
READ_CONFIG(id_dfr0,           isar.id_dfr0)
READ_CONFIG(id_dfr1,           isar.id_dfr1)
READ_CONFIG(id_isar0,          isar.id_isar0)
READ_CONFIG(id_isar1,          isar.id_isar1)
READ_CONFIG(id_isar2,          isar.id_isar2)
READ_CONFIG(id_isar3,          isar.id_isar3)
READ_CONFIG(id_isar4,          isar.id_isar4)
READ_CONFIG(id_isar5,          isar.id_isar5)
READ_CONFIG(id_isar6,          isar.id_isar6)
READ_CONFIG(id_mmfr0,          isar.id_mmfr0)
READ_CONFIG(id_mmfr1,          isar.id_mmfr1)
READ_CONFIG(id_mmfr2,          isar.id_mmfr2)
READ_CONFIG(id_mmfr3,          isar.id_mmfr3)
READ_CONFIG(id_mmfr4,          isar.id_mmfr4)
READ_CONFIG(id_mmfr5,          isar.id_mmfr5)
READ_CONFIG(id_pfr0,           isar.id_pfr0)
READ_CONFIG(id_pfr1,           isar.id_pfr1)
READ_CONFIG(id_pfr2,           isar.id_pfr2)
READ_CONFIG(midr,              midr)
READ_CONFIG(mvfr0_el1,         isar.mvfr0)
READ_CONFIG(mvfr1_el1,         isar.mvfr1)
READ_CONFIG(mvfr2_el1,         isar.mvfr2)
READ_CONFIG(dbgdidr,           isar.dbgdidr)
READ_CONFIG(revidr_el1,        revidr)
READ_CONFIG(mpuir,             mpuir)
READ_CONFIG(hmpuir,            hmpuir)

ARMCPRegInfo common_registers[] = {
    // The params are:  name              cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(ACTLR,            15,   0,   1,   0,   1,   1, RW)  // Auxiliary Control Register
    ARM32_CP_REG_DEFINE(ACTLR2,           15,   0,   1,   0,   3,   1, RW)  // Auxiliary Control Register 2
    ARM32_CP_REG_DEFINE(ADFSR,            15,   0,   5,   1,   0,   1, RW)  // Auxiliary Data Fault Status Register
    ARM32_CP_REG_DEFINE(AIDR,             15,   1,   0,   0,   7,   1, RW)  // Auxiliary ID Register
    ARM32_CP_REG_DEFINE(AIFSR,            15,   0,   5,   1,   1,   1, RW)  // Auxiliary Instruction Fault Status Register
    ARM32_CP_REG_DEFINE(AMAIR0,           15,   0,  10,   3,   0,   1, RW)  // Auxiliary Memory Attribute Indirection Register 0
    ARM32_CP_REG_DEFINE(AMAIR1,           15,   0,  10,   3,   1,   1, RW)  // Auxiliary Memory Attribute Indirection Register 1
    ARM32_CP_REG_DEFINE(AMCFGR,           15,   0,  13,   2,   1,   0, RW)  // Activity Monitors Configuration Register
    ARM32_CP_REG_DEFINE(AMCGCR,           15,   0,  13,   2,   2,   0, RW)  // Activity Monitors Counter Group Configuration Register
    ARM32_CP_REG_DEFINE(AMCNTENCLR0,      15,   0,  13,   2,   4,   0, RW)  // Activity Monitors Count Enable Clear Register 0
    ARM32_CP_REG_DEFINE(AMCNTENCLR1,      15,   0,  13,   3,   0,   0, RW)  // Activity Monitors Count Enable Clear Register 1
    ARM32_CP_REG_DEFINE(AMCNTENSET0,      15,   0,  13,   2,   5,   0, RW)  // Activity Monitors Count Enable Set Register 0
    ARM32_CP_REG_DEFINE(AMCNTENSET1,      15,   0,  13,   3,   1,   0, RW)  // Activity Monitors Count Enable Set Register 1
    ARM32_CP_REG_DEFINE(AMCR,             15,   0,  13,   2,   0,   0, RW)  // Activity Monitors Control Register
    ARM32_CP_REG_DEFINE(AMEVTYPER00,      15,   0,  13,   6,   0,   0, RW)  // Activity Monitors Event Type Registers 0 (0/3)
    ARM32_CP_REG_DEFINE(AMEVTYPER01,      15,   0,  13,   6,   1,   0, RW)  // Activity Monitors Event Type Registers 0 (1/3)
    ARM32_CP_REG_DEFINE(AMEVTYPER02,      15,   0,  13,   6,   2,   0, RW)  // Activity Monitors Event Type Registers 0 (2/3)
    ARM32_CP_REG_DEFINE(AMEVTYPER03,      15,   0,  13,   6,   3,   0, RW)  // Activity Monitors Event Type Registers 0 (3/3)
    ARM32_CP_REG_DEFINE(AMEVTYPER10,      15,   0,  13,  14,   0,   0, RW)  // Activity Monitors Event Type Registers 1 (0/3)
    ARM32_CP_REG_DEFINE(AMEVTYPER11,      15,   0,  13,  14,   1,   0, RW)  // Activity Monitors Event Type Registers 1 (1/3)
    ARM32_CP_REG_DEFINE(AMEVTYPER12,      15,   0,  13,  14,   2,   0, RW)  // Activity Monitors Event Type Registers 1 (2/3)
    ARM32_CP_REG_DEFINE(AMEVTYPER13,      15,   0,  13,  14,   3,   0, RW)  // Activity Monitors Event Type Registers 1 (3/3)
    ARM32_CP_REG_DEFINE(AMUSERENR,        15,   0,  13,   2,   3,   0, RW)  // Activity Monitors User Enable Register
    ARM32_CP_REG_DEFINE(CCSIDR,           15,   1,   0,   0,   0,   1, RW, READFN(ccsidr_el1))  // Current Cache Size ID Register
    ARM32_CP_REG_DEFINE(CCSIDR2,          15,   1,   0,   0,   2,   1, RW, READFN(ccsidr2_el1))  // Current Cache Size ID Register 2
    ARM32_CP_REG_DEFINE(CLIDR,            15,   1,   0,   0,   1,   1, RW, READFN(clidr_el1))  // Cache Level ID Register
    ARM32_CP_REG_DEFINE(CNTKCTL,          15,   0,  14,   1,   0,   1, RW | GTIMER)  // Counter-timer Kernel Control register

    ARM32_CP_REG_DEFINE(CONTEXTIDR,       15,   0,  13,   0,   1,   1, RW, FIELD(cp15.contextidr_ns))  // Context ID Register
    ARM32_CP_REG_DEFINE(CPACR,            15,   0,   1,   0,   2,   1, RW, FIELD(cp15.cpacr_el1))  // Architectural Feature Access Control Register
    ARM32_CP_REG_DEFINE(CSSELR,           15,   2,   0,   0,   0,   1, RW, FIELD(cp15.csselr_ns))  // Cache Size Selection Register
    ARM32_CP_REG_DEFINE(CTR,              15,   0,   0,   0,   1,   1, RW, READFN(ctr_el0))  // Cache Type Register
    ARM32_CP_REG_DEFINE(DACR,             15,   0,   3,   0,   0,   1, RW, FIELD(cp15.dacr_ns))  // Domain Access Control Register
    ARM32_CP_REG_DEFINE(DBGAUTHSTATUS,    14,   0,   7,  14,   6,   1, RW)  // Debug Authentication Status register
    ARM32_CP_REG_DEFINE(DBGBCR0,          14,   0,   0,   0,   5,   1, RW, FIELD(cp15.dbgbcr[0]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR1,          14,   0,   0,   1,   5,   1, RW, FIELD(cp15.dbgbcr[1]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR2,          14,   0,   0,   2,   5,   1, RW, FIELD(cp15.dbgbcr[2]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR3,          14,   0,   0,   3,   5,   1, RW, FIELD(cp15.dbgbcr[3]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR4,          14,   0,   0,   4,   5,   1, RW, FIELD(cp15.dbgbcr[4]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR5,          14,   0,   0,   5,   5,   1, RW, FIELD(cp15.dbgbcr[5]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR6,          14,   0,   0,   6,   5,   1, RW, FIELD(cp15.dbgbcr[6]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR7,          14,   0,   0,   7,   5,   1, RW, FIELD(cp15.dbgbcr[7]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR8,          14,   0,   0,   8,   5,   1, RW, FIELD(cp15.dbgbcr[8]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR9,          14,   0,   0,   9,   5,   1, RW, FIELD(cp15.dbgbcr[9]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR10,         14,   0,   0,  10,   5,   1, RW, FIELD(cp15.dbgbcr[10]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR11,         14,   0,   0,  11,   5,   1, RW, FIELD(cp15.dbgbcr[11]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR12,         14,   0,   0,  12,   5,   1, RW, FIELD(cp15.dbgbcr[12]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR13,         14,   0,   0,  13,   5,   1, RW, FIELD(cp15.dbgbcr[13]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR14,         14,   0,   0,  14,   5,   1, RW, FIELD(cp15.dbgbcr[14]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBCR15,         14,   0,   0,  15,   5,   1, RW, FIELD(cp15.dbgbcr[15]))  // Debug Breakpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGBVR0,          14,   0,   0,   0,   4,   1, RW, FIELD(cp15.dbgbvr[0]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR1,          14,   0,   0,   1,   4,   1, RW, FIELD(cp15.dbgbvr[1]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR2,          14,   0,   0,   2,   4,   1, RW, FIELD(cp15.dbgbvr[2]))  // Debug Breakpoint Value Registers
    // The params are:  name              cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(DBGBVR3,          14,   0,   0,   3,   4,   1, RW, FIELD(cp15.dbgbvr[3]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR4,          14,   0,   0,   4,   4,   1, RW, FIELD(cp15.dbgbvr[4]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR5,          14,   0,   0,   5,   4,   1, RW, FIELD(cp15.dbgbvr[5]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR6,          14,   0,   0,   6,   4,   1, RW, FIELD(cp15.dbgbvr[6]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR7,          14,   0,   0,   7,   4,   1, RW, FIELD(cp15.dbgbvr[7]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR8,          14,   0,   0,   8,   4,   1, RW, FIELD(cp15.dbgbvr[8]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR9,          14,   0,   0,   9,   4,   1, RW, FIELD(cp15.dbgbvr[9]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR10,         14,   0,   0,  10,   4,   1, RW, FIELD(cp15.dbgbvr[10]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR11,         14,   0,   0,  11,   4,   1, RW, FIELD(cp15.dbgbvr[11]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR12,         14,   0,   0,  12,   4,   1, RW, FIELD(cp15.dbgbvr[12]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR13,         14,   0,   0,  13,   4,   1, RW, FIELD(cp15.dbgbvr[13]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR14,         14,   0,   0,  14,   4,   1, RW, FIELD(cp15.dbgbvr[14]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBVR15,         14,   0,   0,  15,   4,   1, RW, FIELD(cp15.dbgbvr[15]))  // Debug Breakpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR0,         14,   0,   1,   0,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR1,         14,   0,   1,   1,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR2,         14,   0,   1,   2,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR3,         14,   0,   1,   3,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR4,         14,   0,   1,   4,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR5,         14,   0,   1,   5,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR6,         14,   0,   1,   6,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR7,         14,   0,   1,   7,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR8,         14,   0,   1,   8,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR9,         14,   0,   1,   9,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR10,        14,   0,   1,  10,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR11,        14,   0,   1,  11,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR12,        14,   0,   1,  12,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR13,        14,   0,   1,  13,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR14,        14,   0,   1,  14,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGBXVR15,        14,   0,   1,  15,   1,   1, RW)  // Debug Breakpoint Extended Value Registers
    ARM32_CP_REG_DEFINE(DBGCLAIMCLR,      14,   0,   7,   9,   6,   1, RW)  // Debug CLAIM Tag Clear register
    ARM32_CP_REG_DEFINE(DBGCLAIMSET,      14,   0,   7,   8,   6,   1, RW)  // Debug CLAIM Tag Set register
    ARM32_CP_REG_DEFINE(DBGDCCINT,        14,   0,   0,   2,   0,   1, RW)  // DCC Interrupt Enable Register
    ARM32_CP_REG_DEFINE(DBGDEVID,         14,   0,   7,   2,   7,   1, RW)  // Debug Device ID register 0
    ARM32_CP_REG_DEFINE(DBGDEVID1,        14,   0,   7,   1,   7,   1, RW)  // Debug Device ID register 1
    ARM32_CP_REG_DEFINE(DBGDEVID2,        14,   0,   7,   0,   7,   1, RW)  // Debug Device ID register 2
    ARM32_CP_REG_DEFINE(DBGDIDR,          14,   0,   0,   0,   0,   0, RO, READFN(dbgdidr))  // Debug ID Register
    ARM32_CP_REG_DEFINE(DBGDRAR_32,       14,   0,   1,   0,   0,   0, RW)  // Debug ROM Address Register
    ARM32_CP_REG_DEFINE(DBGDSAR_32,       14,   0,   2,   0,   0,   0, RW)  // Debug Self Address Register
    ARM32_CP_REG_DEFINE(DBGDSCRext,       14,   0,   0,   2,   2,   1, RW, FIELD(cp15.mdscr_el1))  // Debug Status and Control Register, External View
    ARM32_CP_REG_DEFINE(DBGDSCRint,       14,   0,   0,   1,   0,   0, RW, FIELD(cp15.mdscr_el1))  // Debug Status and Control Register, Internal ViewAArch32 System Registers
    ARM32_CP_REG_DEFINE(DBGDTRRXext,      14,   0,   0,   0,   2,   1, RW)  // Debug OS Lock Data Transfer Register, Receive, External View
    ARM32_CP_REG_DEFINE(DBGDTRRXint,      14,   0,   0,   5,   0,   0, RW)  // Debug Data Transfer Register, Receive
    ARM32_CP_REG_DEFINE(DBGDTRTXext,      14,   0,   0,   3,   2,   1, RW)  // Debug OS Lock Data Transfer Register, Transmit
    ARM32_CP_REG_DEFINE(DBGOSDLR,         14,   0,   1,   3,   4,   1, RW, FIELD(cp15.osdlr_el1))  // Debug OS Double Lock Register
    ARM32_CP_REG_DEFINE(DBGOSECCR,        14,   0,   0,   6,   2,   1, RW)  // Debug OS Lock Exception Catch Control Register
    ARM32_CP_REG_DEFINE(DBGOSLAR,         14,   0,   1,   0,   4,   1, RW)  // Debug OS Lock Access Register
    ARM32_CP_REG_DEFINE(DBGOSLSR,         14,   0,   1,   1,   4,   1, RW, FIELD(cp15.oslsr_el1))  // Debug OS Lock Status Register
    ARM32_CP_REG_DEFINE(DBGPRCR,          14,   0,   1,   4,   4,   1, RW)  // Debug Power Control Register
    ARM32_CP_REG_DEFINE(DBGVCR,           14,   0,   0,   7,   0,   1, RW)  // Debug Vector Catch Register
    ARM32_CP_REG_DEFINE(DBGWCR0,          14,   0,   0,   0,   7,   1, RW, FIELD(cp15.dbgwcr[0]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR1,          14,   0,   0,   1,   7,   1, RW, FIELD(cp15.dbgwcr[1]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR2,          14,   0,   0,   2,   7,   1, RW, FIELD(cp15.dbgwcr[2]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR3,          14,   0,   0,   3,   7,   1, RW, FIELD(cp15.dbgwcr[3]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR4,          14,   0,   0,   4,   7,   1, RW, FIELD(cp15.dbgwcr[4]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR5,          14,   0,   0,   5,   7,   1, RW, FIELD(cp15.dbgwcr[5]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR6,          14,   0,   0,   6,   7,   1, RW, FIELD(cp15.dbgwcr[6]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR7,          14,   0,   0,   7,   7,   1, RW, FIELD(cp15.dbgwcr[7]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR8,          14,   0,   0,   8,   7,   1, RW, FIELD(cp15.dbgwcr[8]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR9,          14,   0,   0,   9,   7,   1, RW, FIELD(cp15.dbgwcr[9]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR10,         14,   0,   0,  10,   7,   1, RW, FIELD(cp15.dbgwcr[10]))  // Debug Watchpoint Control Registers
    // The params are:  name              cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(DBGWCR11,         14,   0,   0,  11,   7,   1, RW, FIELD(cp15.dbgwcr[11]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR12,         14,   0,   0,  12,   7,   1, RW, FIELD(cp15.dbgwcr[12]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR13,         14,   0,   0,  13,   7,   1, RW, FIELD(cp15.dbgwcr[13]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR14,         14,   0,   0,  14,   7,   1, RW, FIELD(cp15.dbgwcr[14]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWCR15,         14,   0,   0,  15,   7,   1, RW, FIELD(cp15.dbgwcr[15]))  // Debug Watchpoint Control Registers
    ARM32_CP_REG_DEFINE(DBGWFAR,          14,   0,   0,   6,   0,   1, RW)  // Debug Watchpoint Fault Address Register
    ARM32_CP_REG_DEFINE(DBGWVR0,          14,   0,   0,   0,   6,   1, RW, FIELD(cp15.dbgwvr[0]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR1,          14,   0,   0,   1,   6,   1, RW, FIELD(cp15.dbgwvr[1]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR2,          14,   0,   0,   2,   6,   1, RW, FIELD(cp15.dbgwvr[2]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR3,          14,   0,   0,   3,   6,   1, RW, FIELD(cp15.dbgwvr[3]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR4,          14,   0,   0,   4,   6,   1, RW, FIELD(cp15.dbgwvr[4]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR5,          14,   0,   0,   5,   6,   1, RW, FIELD(cp15.dbgwvr[5]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR6,          14,   0,   0,   6,   6,   1, RW, FIELD(cp15.dbgwvr[6]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR7,          14,   0,   0,   7,   6,   1, RW, FIELD(cp15.dbgwvr[7]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR8,          14,   0,   0,   8,   6,   1, RW, FIELD(cp15.dbgwvr[8]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR9,          14,   0,   0,   9,   6,   1, RW, FIELD(cp15.dbgwvr[9]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR10,         14,   0,   0,  10,   6,   1, RW, FIELD(cp15.dbgwvr[10]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR11,         14,   0,   0,  11,   6,   1, RW, FIELD(cp15.dbgwvr[11]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR12,         14,   0,   0,  12,   6,   1, RW, FIELD(cp15.dbgwvr[12]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR13,         14,   0,   0,  13,   6,   1, RW, FIELD(cp15.dbgwvr[13]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR14,         14,   0,   0,  14,   6,   1, RW, FIELD(cp15.dbgwvr[14]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DBGWVR15,         14,   0,   0,  15,   6,   1, RW, FIELD(cp15.dbgwvr[15]))  // Debug Watchpoint Value Registers
    ARM32_CP_REG_DEFINE(DFAR,             15,   0,   6,   0,   0,   1, RW, FIELD(cp15.dfar_ns))  // Data Fault Address Register
    ARM32_CP_REG_DEFINE(DFSR,             15,   0,   5,   0,   0,   1, RW, FIELD(cp15.dfsr_ns))  // Data Fault Status Register
    ARM32_CP_REG_DEFINE(DISR,             15,   0,  12,   1,   1,   1, RW, FIELD(cp15.disr_el1))  // Deferred Interrupt Status Register
    ARM32_CP_REG_DEFINE(DLR,              15,   3,   4,   5,   1,   0, RW)  // Debug Link Register
    ARM32_CP_REG_DEFINE(DSPSR,            15,   3,   4,   5,   0,   0, RW)  // Debug Saved Program Status Register
    ARM32_CP_REG_DEFINE(ERRIDR,           15,   0,   5,   3,   0,   0, RW)  // Error Record ID Register
    ARM32_CP_REG_DEFINE(ERRSELR,          15,   0,   5,   3,   1,   0, RW)  // Error Record Select Register
    ARM32_CP_REG_DEFINE(ERXADDR,          15,   0,   5,   4,   3,   0, RW)  // Selected Error Record Address Register
    ARM32_CP_REG_DEFINE(ERXADDR2,         15,   0,   5,   4,   7,   0, RW)  // Selected Error Record Address Register 2
    ARM32_CP_REG_DEFINE(ERXCTLR,          15,   0,   5,   4,   1,   0, RW)  // Selected Error Record Control Register
    ARM32_CP_REG_DEFINE(ERXCTLR2,         15,   0,   5,   4,   5,   0, RW)  // Selected Error Record Control Register 2
    ARM32_CP_REG_DEFINE(ERXFR,            15,   0,   5,   4,   0,   0, RW)  // Selected Error Record Feature Register
    ARM32_CP_REG_DEFINE(ERXFR2,           15,   0,   5,   4,   4,   0, RW)  // Selected Error Record Feature Register 2
    ARM32_CP_REG_DEFINE(ERXMISC0,         15,   0,   5,   5,   0,   0, RW)  // Selected Error Record Miscellaneous Register 0
    ARM32_CP_REG_DEFINE(ERXMISC1,         15,   0,   5,   5,   1,   0, RW)  // Selected Error Record Miscellaneous Register 1
    ARM32_CP_REG_DEFINE(ERXMISC2,         15,   0,   5,   5,   4,   0, RW)  // Selected Error Record Miscellaneous Register 2
    ARM32_CP_REG_DEFINE(ERXMISC3,         15,   0,   5,   5,   5,   0, RW)  // Selected Error Record Miscellaneous Register 3
    ARM32_CP_REG_DEFINE(ERXMISC4,         15,   0,   5,   5,   2,   0, RW)  // Selected Error Record Miscellaneous Register 4
    ARM32_CP_REG_DEFINE(ERXMISC5,         15,   0,   5,   5,   3,   0, RW)  // Selected Error Record Miscellaneous Register 5
    ARM32_CP_REG_DEFINE(ERXMISC6,         15,   0,   5,   5,   6,   0, RW)  // Selected Error Record Miscellaneous Register 6
    ARM32_CP_REG_DEFINE(ERXMISC7,         15,   0,   5,   5,   7,   0, RW)  // Selected Error Record Miscellaneous Register 7
    ARM32_CP_REG_DEFINE(ERXSTATUS,        15,   0,   5,   4,   2,   0, RW)  // Selected Error Record Primary Status RegisterAArch32 System Registers
    ARM32_CP_REG_DEFINE(FCSEIDR,          15,   0,  13,   0,   0,   1, RW, FIELD(cp15.fcseidr_ns))  // FCSE Process ID register
    ARM32_CP_REG_DEFINE(HACR,             15,   4,   1,   1,   7,   2, RW)  // Hyp Auxiliary Configuration Register
    ARM32_CP_REG_DEFINE(HACTLR,           15,   4,   1,   0,   1,   2, RW)  // Hyp Auxiliary Control Register
    ARM32_CP_REG_DEFINE(HACTLR2,          15,   4,   1,   0,   3,   2, RW)  // Hyp Auxiliary Control Register 2
    ARM32_CP_REG_DEFINE(HADFSR,           15,   4,   5,   1,   0,   2, RW)  // Hyp Auxiliary Data Fault Status Register
    ARM32_CP_REG_DEFINE(HAIFSR,           15,   4,   5,   1,   1,   2, RW)  // Hyp Auxiliary Instruction Fault Status Register
    ARM32_CP_REG_DEFINE(HAMAIR0,          15,   4,  10,   3,   0,   2, RW)  // Hyp Auxiliary Memory Attribute Indirection Register 0
    ARM32_CP_REG_DEFINE(HAMAIR1,          15,   4,  10,   3,   1,   2, RW)  // Hyp Auxiliary Memory Attribute Indirection Register 1
    ARM32_CP_REG_DEFINE(HCPTR,            15,   4,   1,   1,   2,   2, RW, FIELD(cp15.cptr_el[2]))  // Hyp Architectural Feature Trap Register
    ARM32_CP_REG_DEFINE(HCR,              15,   4,   1,   1,   0,   2, RW, FIELD(cp15.hcr_el2), RESETVALUE(0x2)) // Hyp Configuration Register
    ARM32_CP_REG_DEFINE(HCR2,             15,   4,   1,   1,   4,   2, RW)  // Hyp Configuration Register 2
    ARM32_CP_REG_DEFINE(HDCR,             15,   4,   1,   1,   1,   2, RW, FIELD(cp15.mdcr_el2), RESETVALUE(0x4))  // Hyp Debug Control Register
    ARM32_CP_REG_DEFINE(HDFAR,            15,   4,   6,   0,   0,   2, RW, FIELD(cp15.dfar_s))  // Hyp Data Fault Address Register
    ARM32_CP_REG_DEFINE(HIFAR,            15,   4,   6,   0,   2,   2, RW, FIELD(cp15.ifar_s))  // Hyp Instruction F ault Address Register
    ARM32_CP_REG_DEFINE(HMAIR0,           15,   4,  10,   2,   0,   2, RW, FIELD(cp15.hmair0))  // Hyp Memory Attribute Indirection Register 0
    ARM32_CP_REG_DEFINE(HMAIR1,           15,   4,  10,   2,   1,   2, RW, FIELD(cp15.hmair1))  // Hyp Memory Attribute Indirection Register 1
    // The params are:  name              cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(HPFAR,            15,   4,   6,   0,   4,   2, RW, FIELD(cp15.hpfar_el2))  // Hyp IP A Fault Address Register
    ARM32_CP_REG_DEFINE(HRMR,             15,   4,  12,   0,   2,   0, RW)  // Hyp Reset Management Register
    ARM32_CP_REG_DEFINE(HSCTLR,           15,   4,   1,   0,   0,   2, RW, FIELD(cp15.hsctlr))  // Hyp System Control Register
    ARM32_CP_REG_DEFINE(HSR,              15,   4,   5,   2,   0,   2, RW, FIELD(cp15.hsr))  // Hyp Syndrome Register
    ARM32_CP_REG_DEFINE(HSTR,             15,   4,   1,   1,   3,   2, RW, FIELD(cp15.hstr_el2))  // Hyp System Trap Register
    ARM32_CP_REG_DEFINE(HTCR,             15,   4,   2,   0,   2,   2, RW)  // Hyp Translation Control Register
    ARM32_CP_REG_DEFINE(HTPIDR,           15,   4,  13,   0,   2,   2, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.tpidr_el[2]))  // Hyp Software Thread ID Register
    ARM32_CP_REG_DEFINE(HTRFCR,           15,   4,   1,   2,   1,   2, RW)  // Hyp Trace Filter Control Register
    ARM32_CP_REG_DEFINE(HVBAR,            15,   4,  12,   0,   0,   2, RW, FIELD(cp15.hvbar))  // Hyp Vector Base Address Register
    // For every 32-bit ICC_* register except ICC_MCTLR, ICC_MGRPEN1 and ICC_MSRE there's also an eqivalent ICV_* register with the same encoding.
    ARM32_CP_REG_DEFINE(ICC_AP0R0,        15,   0,  12,   8,   4,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Active Priorities Group 0 Registers
    ARM32_CP_REG_DEFINE(ICC_AP0R1,        15,   0,  12,   8,   5,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Active Priorities Group 0 Registers
    ARM32_CP_REG_DEFINE(ICC_AP0R2,        15,   0,  12,   8,   6,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Active Priorities Group 0 Registers
    ARM32_CP_REG_DEFINE(ICC_AP0R3,        15,   0,  12,   8,   7,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Active Priorities Group 0 Registers
    ARM32_CP_REG_DEFINE(ICC_AP1R0,        15,   0,  12,   9,   0,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Active Priorities Group 1 Registers
    ARM32_CP_REG_DEFINE(ICC_AP1R1,        15,   0,  12,   9,   1,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Active Priorities Group 1 Registers
    ARM32_CP_REG_DEFINE(ICC_AP1R2,        15,   0,  12,   9,   2,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Active Priorities Group 1 Registers
    ARM32_CP_REG_DEFINE(ICC_AP1R3,        15,   0,  12,   9,   3,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Active Priorities Group 1 Registers
    ARM32_CP_REG_DEFINE(ICC_BPR0,         15,   0,  12,   8,   3,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Binary Point Register 0
    ARM32_CP_REG_DEFINE(ICC_BPR1,         15,   0,  12,  12,   3,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Binary Point Register 1
    ARM32_CP_REG_DEFINE(ICC_CTLR,         15,   0,  12,  12,   4,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Control Register
    ARM32_CP_REG_DEFINE(ICC_DIR,          15,   0,  12,  11,   1,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Deactivate Interrupt RegisterAArch32 System Registers
    ARM32_CP_REG_DEFINE(ICC_EOIR0,        15,   0,  12,   8,   1,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller End Of Interrupt Register 0
    ARM32_CP_REG_DEFINE(ICC_EOIR1,        15,   0,  12,  12,   1,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller End Of Interrupt Register 1
    ARM32_CP_REG_DEFINE(ICC_HPPIR0,       15,   0,  12,   8,   2,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Highest Priority Pending Interrupt Register 0
    ARM32_CP_REG_DEFINE(ICC_HPPIR1,       15,   0,  12,  12,   2,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Highest Priority Pending Interrupt Register 1
    ARM32_CP_REG_DEFINE(ICC_HSRE,         15,   4,  12,   9,   5,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Hyp System Register Enable register
    ARM32_CP_REG_DEFINE(ICC_IAR0,         15,   0,  12,   8,   0,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Interrupt Acknowledge Register 0
    ARM32_CP_REG_DEFINE(ICC_IAR1,         15,   0,  12,  12,   0,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Interrupt Acknowledge Register 1
    ARM32_CP_REG_DEFINE(ICC_IGRPEN0,      15,   0,  12,  12,   6,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Interrupt Group 0 Enable register
    ARM32_CP_REG_DEFINE(ICC_IGRPEN1,      15,   0,  12,  12,   7,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Interrupt Group 1 Enable register
    ARM32_CP_REG_DEFINE(ICC_MCTLR,        15,   6,  12,  12,   4,   3, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Monitor Control Register
    ARM32_CP_REG_DEFINE(ICC_MGRPEN1,      15,   6,  12,  12,   7,   3, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Monitor Interrupt Group 1 Enable register
    ARM32_CP_REG_DEFINE(ICC_MSRE,         15,   6,  12,  12,   5,   3, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Monitor System Register Enable register
    ARM32_CP_REG_DEFINE(ICC_PMR,          15,   0,   4,   6,   0,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Interrupt Priority Mask Register
    ARM32_CP_REG_DEFINE(ICC_RPR,          15,   0,  12,  11,   3,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Running Priority Register
    ARM32_CP_REG_DEFINE(ICC_SRE,          15,   0,  12,  12,   5,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller System Register Enable register
    ARM32_CP_REG_DEFINE(ICH_AP0R0,        15,   4,  12,   8,   0,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Hyp Active Priorities Group 0 Registers
    ARM32_CP_REG_DEFINE(ICH_AP0R1,        15,   4,  12,   8,   1,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Hyp Active Priorities Group 0 Registers
    ARM32_CP_REG_DEFINE(ICH_AP0R2,        15,   4,  12,   8,   2,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Hyp Active Priorities Group 0 Registers
    ARM32_CP_REG_DEFINE(ICH_AP0R3,        15,   4,  12,   8,   3,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Hyp Active Priorities Group 0 Registers
    ARM32_CP_REG_DEFINE(ICH_AP1R0,        15,   4,  12,   9,   0,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Hyp Active Priorities Group 1 Registers
    ARM32_CP_REG_DEFINE(ICH_AP1R1,        15,   4,  12,   9,   1,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Hyp Active Priorities Group 1 Registers
    ARM32_CP_REG_DEFINE(ICH_AP1R2,        15,   4,  12,   9,   2,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Hyp Active Priorities Group 1 Registers
    ARM32_CP_REG_DEFINE(ICH_AP1R3,        15,   4,  12,   9,   3,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Hyp Active Priorities Group 1 Registers
    ARM32_CP_REG_DEFINE(ICH_EISR,         15,   4,  12,  11,   3,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller End of Interrupt Status Register
    ARM32_CP_REG_DEFINE(ICH_ELRSR,        15,   4,  12,  11,   5,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Empty List Register Status Register
    ARM32_CP_REG_DEFINE(ICH_HCR,          15,   4,  12,  11,   0,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Hyp Control Register
    ARM32_CP_REG_DEFINE(ICH_LR0,          15,   4,  12,  12,   0,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR1,          15,   4,  12,  12,   1,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR2,          15,   4,  12,  12,   2,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR3,          15,   4,  12,  12,   3,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR4,          15,   4,  12,  12,   4,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR5,          15,   4,  12,  12,   5,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR6,          15,   4,  12,  12,   6,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR7,          15,   4,  12,  12,   7,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR8,          15,   4,  12,  13,   0,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR9,          15,   4,  12,  13,   1,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR10,         15,   4,  12,  13,   2,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    // The params are:  name              cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(ICH_LR11,         15,   4,  12,  13,   3,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR12,         15,   4,  12,  13,   4,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR13,         15,   4,  12,  13,   5,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR14,         15,   4,  12,  13,   6,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LR15,         15,   4,  12,  13,   7,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC0,         15,   4,  12,  14,   0,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC1,         15,   4,  12,  14,   1,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC2,         15,   4,  12,  14,   2,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC3,         15,   4,  12,  14,   3,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC4,         15,   4,  12,  14,   4,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC5,         15,   4,  12,  14,   5,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC6,         15,   4,  12,  14,   6,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC7,         15,   4,  12,  14,   7,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC8,         15,   4,  12,  15,   0,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC9,         15,   4,  12,  15,   1,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC10,        15,   4,  12,  15,   2,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC11,        15,   4,  12,  15,   3,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC12,        15,   4,  12,  15,   4,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC13,        15,   4,  12,  15,   5,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC14,        15,   4,  12,  15,   6,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_LRC15,        15,   4,  12,  15,   7,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller List Registers
    ARM32_CP_REG_DEFINE(ICH_MISR,         15,   4,  12,  11,   2,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Maintenance Interrupt State Register
    ARM32_CP_REG_DEFINE(ICH_VMCR,         15,   4,  12,  11,   7,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Virtual Machine Control Register
    ARM32_CP_REG_DEFINE(ICH_VTR,          15,   4,  12,  11,   1,   2, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller VGIC Type Register
    ARM32_CP_REG_DEFINE(ID_AFR0,          15,   0,   0,   1,   3,   1, RW, READFN(id_afr0))  // Auxiliary Feature Register 0
    ARM32_CP_REG_DEFINE(ID_DFR0,          15,   0,   0,   1,   2,   1, RW, READFN(id_dfr0))  // Debug Feature Register 0
    ARM32_CP_REG_DEFINE(ID_DFR1,          15,   0,   0,   3,   5,   1, RW, READFN(id_dfr1))  // Debug Feature Register 1
    ARM32_CP_REG_DEFINE(ID_ISAR0,         15,   0,   0,   2,   0,   1, RW, READFN(id_isar0))  // Instruction Set Attribute Register 0
    ARM32_CP_REG_DEFINE(ID_ISAR1,         15,   0,   0,   2,   1,   1, RW, READFN(id_isar1))  // Instruction Set Attribute Register 1
    ARM32_CP_REG_DEFINE(ID_ISAR2,         15,   0,   0,   2,   2,   1, RW, READFN(id_isar2))  // Instruction Set Attribute Register 2
    ARM32_CP_REG_DEFINE(ID_ISAR3,         15,   0,   0,   2,   3,   1, RW, READFN(id_isar3))  // Instruction Set Attribute Register 3
    ARM32_CP_REG_DEFINE(ID_ISAR4,         15,   0,   0,   2,   4,   1, RW, READFN(id_isar4))  // Instruction Set Attribute Register 4
    ARM32_CP_REG_DEFINE(ID_ISAR5,         15,   0,   0,   2,   5,   1, RW, READFN(id_isar5))  // Instruction Set Attribute Register 5
    ARM32_CP_REG_DEFINE(ID_ISAR6,         15,   0,   0,   2,   7,   1, RW, READFN(id_isar6))  // Instruction Set Attribute Register 6
    ARM32_CP_REG_DEFINE(ID_MMFR0,         15,   0,   0,   1,   4,   1, RW, READFN(id_mmfr0))  // Memory Model Feature Register 0
    ARM32_CP_REG_DEFINE(ID_MMFR1,         15,   0,   0,   1,   5,   1, RW, READFN(id_mmfr1))  // Memory Model Feature Register 1
    ARM32_CP_REG_DEFINE(ID_MMFR2,         15,   0,   0,   1,   6,   1, RW, READFN(id_mmfr2))  // Memory Model Feature Register 2
    ARM32_CP_REG_DEFINE(ID_MMFR3,         15,   0,   0,   1,   7,   1, RW, READFN(id_mmfr3))  // Memory Model Feature Register 3
    ARM32_CP_REG_DEFINE(ID_MMFR4,         15,   0,   0,   2,   6,   1, RW, READFN(id_mmfr4))  // Memory Model Feature Register 4
    ARM32_CP_REG_DEFINE(ID_MMFR5,         15,   0,   0,   3,   6,   1, RW, READFN(id_mmfr5))  // Memory Model Feature Register 5
    ARM32_CP_REG_DEFINE(ID_PFR0,          15,   0,   0,   1,   0,   1, RW, READFN(id_pfr0))  // Processor Feature Register 0
    ARM32_CP_REG_DEFINE(ID_PFR1,          15,   0,   0,   1,   1,   1, RW, READFN(id_pfr1))  // Processor Feature Register 1
    ARM32_CP_REG_DEFINE(ID_PFR2,          15,   0,   0,   3,   4,   1, RW, READFN(id_pfr2))  // Processor Feature Register 2
    ARM32_CP_REG_DEFINE(IFAR,             15,   0,   6,   0,   2,   1, RW, FIELD(cp15.ifar_ns)) // Instruction Fault Address Register
    ARM32_CP_REG_DEFINE(IFSR,             15,   0,   5,   0,   1,   1, RW, FIELD(cp15.ifsr_ns))  // Instruction Fault Status Register
    ARM32_CP_REG_DEFINE(ISR,              15,   0,  12,   1,   0,   1, RW)  // Interrupt Status Register
    ARM32_CP_REG_DEFINE(JIDR,             14,   7,   0,   0,   0,   0, RW)  // Jazelle ID Register
    ARM32_CP_REG_DEFINE(JMCR,             14,   7,   2,   0,   0,   0, RW)  // Jazelle Main Configuration Register
    ARM32_CP_REG_DEFINE(JOSCR,            14,   7,   1,   0,   0,   0, RW)  // Jazelle OS Control Register
    ARM32_CP_REG_DEFINE(MAIR0,            15,   0,  10,   2,   0,   1, RW, FIELD(cp15.mair0_ns))  // Memory Attribute Indirection Register 0
    ARM32_CP_REG_DEFINE(MAIR1,            15,   0,  10,   2,   1,   1, RW, FIELD(cp15.mair1_ns))  // Memory Attribute Indirection Register 1
    ARM32_CP_REG_DEFINE(MIDR,             15,   0,   0,   0,   0,   1, RO, READFN(midr))  // Main ID Register
    ARM32_CP_REG_DEFINE(MPIDR,            15,   0,   0,   0,   5,   1, RO, READFN(mpidr_el1))  // Multiprocessor Affinity RegisterAArch32 System Registers
    ARM32_CP_REG_DEFINE(MIDR_ALIAS,       15,   0,   0,   0,   7,   1, RO, READFN(midr))  // Alias of Main ID Register
    ARM32_CP_REG_DEFINE(NSACR,            15,   0,   1,   1,   2,   1, RW, FIELD(cp15.nsacr), RESETVALUE(0xC00))  // Non-Secure Access Control Register
    ARM32_CP_REG_DEFINE(PAR_32,           15,   0,   7,   4,   0,   1, RW, FIELD(cp15.par_ns))  // Physical Address Register
    ARM32_CP_REG_DEFINE(PMCCFILTR,        15,   0,  14,  15,   7,   0, RW, FIELD(cp15.pmccfiltr_el0))  // Performance Monitors Cycle Count Filter Register
    ARM32_CP_REG_DEFINE(PMCCNTR_32,       15,   0,   9,  13,   0,   0, RW)  // Performance Monitors Cycle Count Register
    ARM32_CP_REG_DEFINE(PMCEID0,          15,   0,   9,  12,   6,   0, RO | ARM_CP_CONST, RESETVALUE(0x6E1FFFDB))  // Performance Monitors Common Event Identification register 0
    ARM32_CP_REG_DEFINE(PMCEID1,          15,   0,   9,  12,   7,   0, RO | ARM_CP_CONST, RESETVALUE(0x0000001E))  // Performance Monitors Common Event Identification register 1
    ARM32_CP_REG_DEFINE(PMCEID2,          15,   0,   9,  14,   4,   0, RO)  // Performance Monitors Common Event Identification register 2
    // The params are:  name              cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(PMCEID3,          15,   0,   9,  14,   5,   0, RO)  // Performance Monitors Common Event Identification register 3
    ARM32_CP_REG_DEFINE(PMCNTENCLR,       15,   0,   9,  12,   2,   0, RW, FIELD(cp15.c9_pmcnten))  // Performance Monitors Count Enable Clear register
    ARM32_CP_REG_DEFINE(PMCNTENSET,       15,   0,   9,  12,   1,   0, RW, FIELD(cp15.c9_pmcnten))  // Performance Monitors Count Enable Set register
    ARM32_CP_REG_DEFINE(PMCR,             15,   0,   9,  12,   0,   0, RW, FIELD(cp15.c9_pmcr))  // Performance Monitors Control Register
    ARM32_CP_REG_DEFINE(PMEVCNTR0,        15,   0,  14,   8,   0,   0, RW, FIELD(cp15.c14_pmevcntr[0]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR1,        15,   0,  14,   8,   1,   0, RW, FIELD(cp15.c14_pmevcntr[1]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR2,        15,   0,  14,   8,   2,   0, RW, FIELD(cp15.c14_pmevcntr[2]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR3,        15,   0,  14,   8,   3,   0, RW, FIELD(cp15.c14_pmevcntr[3]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR4,        15,   0,  14,   8,   4,   0, RW, FIELD(cp15.c14_pmevcntr[4]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR5,        15,   0,  14,   8,   5,   0, RW, FIELD(cp15.c14_pmevcntr[5]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR6,        15,   0,  14,   8,   6,   0, RW, FIELD(cp15.c14_pmevcntr[6]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR7,        15,   0,  14,   8,   7,   0, RW, FIELD(cp15.c14_pmevcntr[7]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR8,        15,   0,  14,   9,   0,   0, RW, FIELD(cp15.c14_pmevcntr[8]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR9,        15,   0,  14,   9,   1,   0, RW, FIELD(cp15.c14_pmevcntr[9]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR10,       15,   0,  14,   9,   2,   0, RW, FIELD(cp15.c14_pmevcntr[10]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR11,       15,   0,  14,   9,   3,   0, RW, FIELD(cp15.c14_pmevcntr[11]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR12,       15,   0,  14,   9,   4,   0, RW, FIELD(cp15.c14_pmevcntr[12]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR13,       15,   0,  14,   9,   5,   0, RW, FIELD(cp15.c14_pmevcntr[13]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR14,       15,   0,  14,   9,   6,   0, RW, FIELD(cp15.c14_pmevcntr[14]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR15,       15,   0,  14,   9,   7,   0, RW, FIELD(cp15.c14_pmevcntr[15]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR16,       15,   0,  14,  10,   0,   0, RW, FIELD(cp15.c14_pmevcntr[16]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR17,       15,   0,  14,  10,   1,   0, RW, FIELD(cp15.c14_pmevcntr[17]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR18,       15,   0,  14,  10,   2,   0, RW, FIELD(cp15.c14_pmevcntr[18]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR19,       15,   0,  14,  10,   3,   0, RW, FIELD(cp15.c14_pmevcntr[19]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR20,       15,   0,  14,  10,   4,   0, RW, FIELD(cp15.c14_pmevcntr[20]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR21,       15,   0,  14,  10,   5,   0, RW, FIELD(cp15.c14_pmevcntr[21]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR22,       15,   0,  14,  10,   6,   0, RW, FIELD(cp15.c14_pmevcntr[22]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR23,       15,   0,  14,  10,   7,   0, RW, FIELD(cp15.c14_pmevcntr[23]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR24,       15,   0,  14,  11,   0,   0, RW, FIELD(cp15.c14_pmevcntr[24]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR25,       15,   0,  14,  11,   1,   0, RW, FIELD(cp15.c14_pmevcntr[25]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR26,       15,   0,  14,  11,   2,   0, RW, FIELD(cp15.c14_pmevcntr[26]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR27,       15,   0,  14,  11,   3,   0, RW, FIELD(cp15.c14_pmevcntr[27]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR28,       15,   0,  14,  11,   4,   0, RW, FIELD(cp15.c14_pmevcntr[28]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR29,       15,   0,  14,  11,   5,   0, RW, FIELD(cp15.c14_pmevcntr[29]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVCNTR30,       15,   0,  14,  11,   6,   0, RW, FIELD(cp15.c14_pmevcntr[30]))  // Performance Monitors Event Count Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER0,       15,   0,  14,  12,   0,   0, RW, FIELD(cp15.c14_pmevtyper[0]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER1,       15,   0,  14,  12,   1,   0, RW, FIELD(cp15.c14_pmevtyper[1]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER2,       15,   0,  14,  12,   2,   0, RW, FIELD(cp15.c14_pmevtyper[2]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER3,       15,   0,  14,  12,   3,   0, RW, FIELD(cp15.c14_pmevtyper[3]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER4,       15,   0,  14,  12,   4,   0, RW, FIELD(cp15.c14_pmevtyper[4]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER5,       15,   0,  14,  12,   5,   0, RW, FIELD(cp15.c14_pmevtyper[5]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER6,       15,   0,  14,  12,   6,   0, RW, FIELD(cp15.c14_pmevtyper[6]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER7,       15,   0,  14,  12,   7,   0, RW, FIELD(cp15.c14_pmevtyper[7]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER8,       15,   0,  14,  13,   0,   0, RW, FIELD(cp15.c14_pmevtyper[8]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER9,       15,   0,  14,  13,   1,   0, RW, FIELD(cp15.c14_pmevtyper[9]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER10,      15,   0,  14,  13,   2,   0, RW, FIELD(cp15.c14_pmevtyper[10]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER11,      15,   0,  14,  13,   3,   0, RW, FIELD(cp15.c14_pmevtyper[11]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER12,      15,   0,  14,  13,   4,   0, RW, FIELD(cp15.c14_pmevtyper[12]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER13,      15,   0,  14,  13,   5,   0, RW, FIELD(cp15.c14_pmevtyper[13]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER14,      15,   0,  14,  13,   6,   0, RW, FIELD(cp15.c14_pmevtyper[14]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER15,      15,   0,  14,  13,   7,   0, RW, FIELD(cp15.c14_pmevtyper[15]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER16,      15,   0,  14,  14,   0,   0, RW, FIELD(cp15.c14_pmevtyper[16]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER17,      15,   0,  14,  14,   1,   0, RW, FIELD(cp15.c14_pmevtyper[17]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER18,      15,   0,  14,  14,   2,   0, RW, FIELD(cp15.c14_pmevtyper[18]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER19,      15,   0,  14,  14,   3,   0, RW, FIELD(cp15.c14_pmevtyper[19]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER22,      15,   0,  14,  14,   4,   0, RW, FIELD(cp15.c14_pmevtyper[20]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER20,      15,   0,  14,  14,   5,   0, RW, FIELD(cp15.c14_pmevtyper[21]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER21,      15,   0,  14,  14,   6,   0, RW, FIELD(cp15.c14_pmevtyper[22]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER23,      15,   0,  14,  14,   7,   0, RW, FIELD(cp15.c14_pmevtyper[23]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER24,      15,   0,  14,  15,   0,   0, RW, FIELD(cp15.c14_pmevtyper[24]))  // Performance Monitors Event Type Registers
    // The params are:  name              cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(PMEVTYPER25,      15,   0,  14,  15,   1,   0, RW, FIELD(cp15.c14_pmevtyper[25]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER26,      15,   0,  14,  15,   2,   0, RW, FIELD(cp15.c14_pmevtyper[26]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER27,      15,   0,  14,  15,   3,   0, RW, FIELD(cp15.c14_pmevtyper[27]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER28,      15,   0,  14,  15,   4,   0, RW, FIELD(cp15.c14_pmevtyper[28]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER29,      15,   0,  14,  15,   5,   0, RW, FIELD(cp15.c14_pmevtyper[29]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMEVTYPER30,      15,   0,  14,  15,   6,   0, RW, FIELD(cp15.c14_pmevtyper[30]))  // Performance Monitors Event Type Registers
    ARM32_CP_REG_DEFINE(PMINTENCLR,       15,   0,   9,  14,   2,   1, RW, FIELD(cp15.c9_pminten))  // Performance Monitors Interrupt Enable Clear register
    ARM32_CP_REG_DEFINE(PMINTENSET,       15,   0,   9,  14,   1,   1, RW, FIELD(cp15.c9_pminten))  // Performance Monitors Interrupt Enable Set register
    ARM32_CP_REG_DEFINE(PMMIR,            15,   0,   9,  14,   6,   1, RW)  // Performance Monitors Machine Identification Register
    ARM32_CP_REG_DEFINE(PMOVSR,           15,   0,   9,  12,   3,   0, RW, FIELD(cp15.c9_pmovsr))  // Performance Monitors Overflow Flag Status Register
    ARM32_CP_REG_DEFINE(PMOVSSET,         15,   0,   9,  14,   3,   0, RW)  // Performance Monitors Overflow Flag Status Set register
    ARM32_CP_REG_DEFINE(PMSELR,           15,   0,   9,  12,   5,   0, RW, FIELD(cp15.c9_pmselr))  // Performance Monitors Event Counter Selection Register
    ARM32_CP_REG_DEFINE(PMSWINC,          15,   0,   9,  12,   4,   0, RW)  // Performance Monitors Software Increment register
    ARM32_CP_REG_DEFINE(PMUSERENR,        15,   0,   9,  14,   0,   0, RW, FIELD(cp15.c9_pmuserenr))  // Performance Monitors User Enable Register
    ARM32_CP_REG_DEFINE(PMXEVCNTR,        15,   0,   9,  13,   2,   0, RW)  // Performance Monitors Selected Event Count Register
    ARM32_CP_REG_DEFINE(PMXEVTYPER,       15,   0,   9,  13,   1,   0, RW)  // Performance Monitors Selected Event Type Register
    ARM32_CP_REG_DEFINE(REVIDR,           15,   0,   0,   0,   6,   1, RO, READFN(revidr_el1))  // Revision ID Register
    ARM32_CP_REG_DEFINE(RMR,              15,   0,  12,   0,   2,   1, RW)  // Reset Management Register
    ARM32_CP_REG_DEFINE(RVBAR,            15,   0,  12,   0,   1,   1, RO, FIELD(cp15.rvbar))  // Reset Vector Base Address Register
    ARM32_CP_REG_DEFINE(SCR,              15,   0,   1,   1,   0,   3, RW, FIELD(cp15.scr_el3))  // Secure Configuration Register
    ARM32_CP_REG_DEFINE(SCTLR,            15,   0,   1,   0,   0,   1, RW, FIELD(cp15.sctlr_ns))  // System Control Register
    ARM32_CP_REG_DEFINE(SDCR,             15,   0,   1,   3,   1,   3, RW)  // Secure Debug Control Register
    ARM32_CP_REG_DEFINE(SDER,             15,   0,   1,   1,   1,   3, RW, FIELD(cp15.sder))  // Secure Debug Enable RegisterAArch32 System Registers
    ARM32_CP_REG_DEFINE(TCMTR,            15,   0,   0,   0,   2,   1, RW, FIELD(cp15.tcm_type))  // TCM Type Register
    ARM32_CP_REG_DEFINE(TLBTR,            15,   0,   0,   0,   3,   1, RW)  // TLB Type Register
    ARM32_CP_REG_DEFINE(TPIDRPRW,         15,   0,  13,   0,   4,   1, RW, FIELD(cp15.tpidrprw_ns))  // PL1 Software Thread ID Register
    ARM32_CP_REG_DEFINE(TPIDRURO,         15,   0,  13,   0,   3,   0, RW, FIELD(cp15.tpidruro_ns))  // PL0 Read-Only Software Thread ID Register
    ARM32_CP_REG_DEFINE(TPIDRURW,         15,   0,  13,   0,   2,   0, RW, FIELD(cp15.tpidrurw_ns))  // PL0 Read/W rite Software Thread ID Register
    ARM32_CP_REG_DEFINE(TRFCR,            15,   0,   1,   2,   1,   1, RW)  // Trace Filter Control Register
    ARM32_CP_REG_DEFINE(TTBCR,            15,   0,   2,   0,   2,   1, RW)  // Translation Table Base Control Register
    ARM32_CP_REG_DEFINE(TTBCR2,           15,   0,   2,   0,   3,   1, RW)  // Translation Table Base Control Register 2
    ARM32_CP_REG_DEFINE(TTBR0_32,         15,   0,   2,   0,   0,   1, RW, FIELD(cp15.ttbr0_ns))  // Translation Table Base Register 0
    ARM32_CP_REG_DEFINE(TTBR1_32,         15,   0,   2,   0,   1,   1, RW, FIELD(cp15.ttbr1_ns))  // Translation Table Base Register 1
    ARM32_CP_REG_DEFINE(VBAR,             15,   0,  12,   0,   0,   1, RW, FIELD(cp15.vbar_ns))  // Vector Base Address Register
    ARM32_CP_REG_DEFINE(VDFSR,            15,   4,   5,   2,   3,   0, RW)  // Virtual SError Exception Syndrome Register
    ARM32_CP_REG_DEFINE(VDISR,            15,   4,  12,   1,   1,   2, RW, FIELD(cp15.vdisr_el2))  // Virtual Deferred Interrupt Status Register
    ARM32_CP_REG_DEFINE(VMPIDR,           15,   4,   0,   0,   5,   2, RW, FIELD(cp15.vmpidr_el2))  // Virtualization Multiprocessor ID Register
    ARM32_CP_REG_DEFINE(VPIDR,            15,   4,   0,   0,   0,   2, RW, FIELD(cp15.vpidr_el2))  // Virtualization Processor ID Register
    ARM32_CP_REG_DEFINE(VTCR,             15,   4,   2,   1,   2,   2, RW, FIELD(cp15.vtcr_el2))  // Virtualization Translation Control Register
    ARM32_CP_REG_DEFINE(VSCTLR,           15,   4,   2,   0,   0,   2, RW)  // Virtualization System Control Register

    // The params are:        name              cp, op1, crm,  el, extra_type, ...
    ARM32_CP_64BIT_REG_DEFINE(AMEVCNTR00,       15,   0,   0,   0, RW)  // Activity Monitors Event Counter Registers 0 (0/3)
    ARM32_CP_64BIT_REG_DEFINE(AMEVCNTR01,       15,   1,   0,   0, RW)  // Activity Monitors Event Counter Registers 0 (1/3)
    ARM32_CP_64BIT_REG_DEFINE(AMEVCNTR02,       15,   2,   0,   0, RW)  // Activity Monitors Event Counter Registers 0 (2/3)
    ARM32_CP_64BIT_REG_DEFINE(AMEVCNTR03,       15,   3,   0,   0, RW)  // Activity Monitors Event Counter Registers 0 (3/3)
    ARM32_CP_64BIT_REG_DEFINE(AMEVCNTR10,       15,   0,   4,   0, RW)  // Activity Monitors Event Counter Registers 1 (0/3)
    ARM32_CP_64BIT_REG_DEFINE(AMEVCNTR11,       15,   1,   4,   0, RW)  // Activity Monitors Event Counter Registers 1 (1/3)
    ARM32_CP_64BIT_REG_DEFINE(AMEVCNTR12,       15,   2,   4,   0, RW)  // Activity Monitors Event Counter Registers 1 (2/3)
    ARM32_CP_64BIT_REG_DEFINE(AMEVCNTR13,       15,   3,   4,   0, RW)  // Activity Monitors Event Counter Registers 1 (3/3)
    ARM32_CP_64BIT_REG_DEFINE(CNTHP_CVAL,       15,   6,  14,   2, RW | GTIMER, RW_FNS(generic_timer_aarch32_64))  // Counter-timer Hyp Physical CompareV alue register
    ARM32_CP_64BIT_REG_DEFINE(CNTPCT,           15,   0,  14,   0, RW | GTIMER, RW_FNS(generic_timer_aarch32_64)) // Counter-timer Physical Count register
    ARM32_CP_64BIT_REG_DEFINE(CNTPCTSS,         15,   8,  14,   0, RW | GTIMER, RW_FNS(generic_timer_aarch32_64))  // Counter-timer Self-Synchronized Physical Count register
    ARM32_CP_64BIT_REG_DEFINE(CNTP_CVAL,        15,   2,  14,   0, RW | GTIMER, RW_FNS(generic_timer_aarch32_64))  // Counter-timer Physical Timer Compare Value register
    ARM32_CP_64BIT_REG_DEFINE(CNTVCT,           15,   1,  14,   0, RW | GTIMER, RW_FNS(generic_timer_aarch32_64))  // Counter-timer Virtual Count register
    ARM32_CP_64BIT_REG_DEFINE(CNTVCTSS,         15,   9,  14,   0, RW | GTIMER, RW_FNS(generic_timer_aarch32_64))  // Counter-timer Self-Synchronized Virtual Count register
    ARM32_CP_64BIT_REG_DEFINE(CNTVOFF,          15,   4,  14,   2, RW | GTIMER, RW_FNS(generic_timer_aarch32_64))  // Counter-timer Virtual Offset register
    ARM32_CP_64BIT_REG_DEFINE(CNTV_CVAL,        15,   3,  14,   0, RW | GTIMER, RW_FNS(generic_timer_aarch32_64))  // Counter-timer Virtual Timer Compare Value register
    ARM32_CP_64BIT_REG_DEFINE(DBGDRAR,          14,   0,   1,   0, RW)  // Debug ROM Address Register
    ARM32_CP_64BIT_REG_DEFINE(DBGDSAR,          14,   0,   2,   0, RW)  // Debug Self Address Register
    ARM32_CP_64BIT_REG_DEFINE(HTTBR,            15,   4,   2,   2, RW)  // Hyp Translation T able Base Register
    ARM32_CP_64BIT_REG_DEFINE(ICC_ASGI1R,       15,   1,  12,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Alias Software Generated Interrupt Group 1 Register
    ARM32_CP_64BIT_REG_DEFINE(ICC_SGI0R,        15,   2,  12,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Software Generated Interrupt Group 0 Register
    ARM32_CP_64BIT_REG_DEFINE(ICC_SGI1R,        15,   0,  12,   1, RW | GIC, RW_FNS(interrupt_cpu_interface))  // Interrupt Controller Software Generated Interrupt Group 1 Register
    ARM32_CP_64BIT_REG_DEFINE(PAR,              15,   0,   7,   1, RW, FIELD(cp15.par_ns))  // Physical Address Register
    ARM32_CP_64BIT_REG_DEFINE(PMCCNTR,          15,   0,   9,   0, RW)  // Performance Monitors Cycle Count Register
    ARM32_CP_64BIT_REG_DEFINE(TTBR0,            15,   0,   2,   1, RW, FIELD(cp15.ttbr0_ns))  // Translation Table Base Register 0
    ARM32_CP_64BIT_REG_DEFINE(TTBR1,            15,   1,   2,   1, RW, FIELD(cp15.ttbr1_ns))  // Translation Table Base Register 1
    ARM32_CP_64BIT_REG_DEFINE(VTTBR,            15,   6,   2,   2, RW, FIELD(cp15.vttbr_el2))  // Virtualization Translation Table Base Register
                                                                                           //
    /*
     * Some registers in the ARM Manuals have the same encodings, but different names.
     * Usually different names are given to distinguish between the different context of
     * the access to the register (for example different PL/EL)
     *
     * The following registers have duplicated encodings (with the existing registers):
     *   CNTHPS_CTL, CNTHPS_CVAL, CNTHPS_TVAL, CNTHV_CTL, CNTHV_CVAL, CNTHV_TVAL,
     *   CNTHVS_CTL, CNTHVS_CVAL, CNTHVS_TVAL, DBGDTRTXint, MVBAR, NMRR, PRRR
     */
};

ARMCPRegInfo aarch32_only_registers[] = {
    ARM32_CP_REG_DEFINE(CNTFRQ,           15,   0,  14,   0,   0,   0, RW | GTIMER, RW_FNS(generic_timer_aarch32_32))  // Counter-timer Frequency register
    ARM32_CP_REG_DEFINE(CNTHCTL,          15,   4,  14,   1,   0,   2, RW | GTIMER, RW_FNS(generic_timer_aarch32_32))  // Counter-timer Hyp Control register
    ARM32_CP_REG_DEFINE(CNTHP_CTL,        15,   4,  14,   2,   1,   2, RW | GTIMER, RW_FNS(generic_timer_aarch32_32))  // Counter-timer Hyp Physical Timer Control register
    ARM32_CP_REG_DEFINE(CNTHP_TVAL,       15,   4,  14,   2,   0,   2, RW | GTIMER, RW_FNS(generic_timer_aarch32_32))  // Counter-timer Hyp Physical Timer Timer Value register
    ARM32_CP_REG_DEFINE(CNTP_CTL,         15,   0,  14,   2,   1,   0, RW | GTIMER, RW_FNS(generic_timer_aarch32_32))  // Counter-timer Physical Timer Control register
    ARM32_CP_REG_DEFINE(CNTP_TVAL,        15,   0,  14,   2,   0,   0, RW | GTIMER, RW_FNS(generic_timer_aarch32_32))  // Counter-timer Physical Timer Timer Value register
    ARM32_CP_REG_DEFINE(CNTV_CTL,         15,   0,  14,   3,   1,   0, RW | GTIMER, RW_FNS(generic_timer_aarch32_32))  // Counter-timer Virtual Timer Control register
    ARM32_CP_REG_DEFINE(CNTV_TVAL,        15,   0,  14,   3,   0,   0, RW | GTIMER, RW_FNS(generic_timer_aarch32_32))  // Counter-timer Virtual Timer Timer Value register
};
/* TLBI helpers */

typedef enum {
    TLBI_IS,
    TLBI_NS,
    TLBI_OS,
} TLBIShareability;

static inline uint16_t tlbi_get_mmu_indexes_mask(CPUState *env, const ARMCPRegInfo *ri)
{
    uint16_t el1_map, el2_map;
    if(arm_is_secure_below_el3(env)) {
        el1_map = ARMMMUIdxBit_SE10_1 | ARMMMUIdxBit_SE10_1_PAN | ARMMMUIdxBit_SE10_0;
        el2_map = ARMMMUIdxBit_SE20_2 | ARMMMUIdxBit_SE20_2_PAN | ARMMMUIdxBit_SE20_0;
    } else {
        el1_map = ARMMMUIdxBit_E10_1 | ARMMMUIdxBit_E10_1_PAN | ARMMMUIdxBit_E10_0;
        el2_map = ARMMMUIdxBit_E20_2 | ARMMMUIdxBit_E20_2_PAN | ARMMMUIdxBit_E20_0;
    }

    //  Fortunately the instruction's min. access EL matches the target EL, e.g. it's 2 for VAE2.
    uint32_t tlbi_target_el = ARM_CP_GET_MIN_EL(ri->type);
    switch(tlbi_target_el) {
        case 1:
            return arm_is_el2_enabled(env) && are_hcr_e2h_and_tge_set(arm_hcr_el2_eff(env)) ? el2_map : el1_map;
        case 2:
            return el2_map;
        case 3:
            return ARMMMUIdxBit_SE3;
        default:
            tlib_assert_not_reached();
    }
}

TLBIShareability tlbi_get_shareability(CPUState *env, const ARMCPRegInfo *ri)
{
    if(strstr(ri->name, "IS") != NULL) {
        return TLBI_IS;
    } else if(strstr(ri->name, "OS") != NULL) {
        return TLBI_OS;
    } else {
        //  The HCR_EL2's FB bit forces inner shareability for EL1.
        if((arm_current_el(env) == 1) && (arm_hcr_el2_eff(env) & HCR_FB)) {
            return TLBI_IS;
        }
        return TLBI_NS;
    }
}

void tlbi_print_stub_logs(CPUState *env, const ARMCPRegInfo *ri)
{
    TLBIShareability tlbi_shareability = tlbi_get_shareability(env, ri);
    if(tlbi_shareability != TLBI_NS) {
        tlib_printf(LOG_LEVEL_DEBUG, "[%s] %s Shareable domain not implemented yet; falling back to normal variant", ri->name,
                    tlbi_shareability == TLBI_IS ? "Inner" : "Outer");
    }
}

WRITE_FUNCTION(64, ic_ialluis, helper_invalidate_dirty_addresses_shared(env))

//  TODO: Implement remaining TLBI instructions.
WRITE_FUNCTION(64, tlbi_flush_all, {
    tlib_printf(LOG_LEVEL_DEBUG, "[%s] Using TLBI stub, forcing full flush", info->name);

    tlb_flush(env, 1, true);
})

WRITE_FUNCTION(64, tlbi_va, {
    tlbi_print_stub_logs(env, info);

    uint64_t pageaddr = sextract64(value << 12, 0, 56);
    uint32_t indexes_mask = tlbi_get_mmu_indexes_mask(env, info);
    tlb_flush_page_masked(env, pageaddr, indexes_mask, true);
})

WRITE_FUNCTION(64, tlbi_vmall, {
    tlbi_print_stub_logs(env, info);

    uint16_t indexes_mask = tlbi_get_mmu_indexes_mask(env, info);
    tlb_flush_masked(env, indexes_mask);
})

ARMCPRegInfo aarch32_instructions[] = {
    // The params are:  name              cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(ATS12NSOPR,       15,   0,   7,   8,   4,   2, WO | INSTRUCTION)  // Address Translate Stages 1 and 2 Non-secure Only PL1 Read
    ARM32_CP_REG_DEFINE(ATS12NSOPW,       15,   0,   7,   8,   5,   2, WO | INSTRUCTION)  // Address Translate Stages 1 and 2 Non-secure Only PL1 Write
    ARM32_CP_REG_DEFINE(ATS12NSOUR,       15,   0,   7,   8,   6,   2, WO | INSTRUCTION)  // Address Translate Stages 1 and 2 Non-secure Only Unprivileged Read
    ARM32_CP_REG_DEFINE(ATS12NSOUW,       15,   0,   7,   8,   7,   2, WO | INSTRUCTION)  // Address Translate Stages 1 and 2 Non-secure Only Unprivileged Write
    ARM32_CP_REG_DEFINE(ATS1CPR,          15,   0,   7,   8,   0,   1, WO | INSTRUCTION, WRITEFN(ats1))  // Address Translate Stage 1 Current state PL1 Read
    ARM32_CP_REG_DEFINE(ATS1CPRP,         15,   0,   7,   9,   0,   1, WO | INSTRUCTION)  // Address Translate Stage 1 Current state PL1 Read PAN
    ARM32_CP_REG_DEFINE(ATS1CPW,          15,   0,   7,   8,   1,   1, WO | INSTRUCTION, WRITEFN(ats1))  // Address Translate Stage 1 Current state PL1 Write
    ARM32_CP_REG_DEFINE(ATS1CPWP,         15,   0,   7,   9,   1,   1, WO | INSTRUCTION)  // Address Translate Stage 1 Current state PL1 Write PAN
    ARM32_CP_REG_DEFINE(ATS1CUR,          15,   0,   7,   8,   2,   1, WO | INSTRUCTION)  // Address Translate Stage 1 Current state Unprivileged Read
    ARM32_CP_REG_DEFINE(ATS1CUW,          15,   0,   7,   8,   3,   1, WO | INSTRUCTION)  // Address Translate Stage 1 Current state Unprivileged Write
    ARM32_CP_REG_DEFINE(ATS1HR,           15,   4,   7,   8,   0,   2, WO | INSTRUCTION)  // Address Translate Stage 1 Hyp mode Read
    ARM32_CP_REG_DEFINE(ATS1HW,           15,   4,   7,   8,   1,   2, WO | INSTRUCTION)  // Address Translate Stage 1 Hyp mode Write
    // TODO: TGAU: #93594 TLB flush shouldn't be necessary, but Linux userspace after fork breaks without this
    ARM32_CP_REG_DEFINE(BPIALL,           15,   0,   7,   5,   6,   1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))  // Branch Predictor Invalidate All
    ARM32_CP_REG_DEFINE(BPIALLIS,         15,   0,   7,   1,   6,   1, WO | INSTRUCTION)  // Branch Predictor Invalidate All, Inner Shareable
    ARM32_CP_REG_DEFINE(BPIMVA,           15,   0,   7,   5,   7,   1, WO | INSTRUCTION)  // Branch Predictor Invalidate by VA
    ARM32_CP_REG_DEFINE(CFPRCTX,          15,   0,   7,   3,   4,   0, WO | INSTRUCTION)  // Control Flow Prediction Restriction by Context
    ARM32_CP_REG_DEFINE(CP15DMB,          15,   0,   7,  10,   5,   0, WO | INSTRUCTION)  // Data Memory Barrier System instruction
    ARM32_CP_REG_DEFINE(CP15DSB,          15,   0,   7,  10,   4,   0, WO | INSTRUCTION)  // Data Synchronization Barrier System instruction
    ARM32_CP_REG_DEFINE(CP15ISB,          15,   0,   7,   5,   4,   0, WO | INSTRUCTION)  // Instruction Synchronization Barrier System instruction
    ARM32_CP_REG_DEFINE(CPPRCTX,          15,   0,   7,   3,   7,   0, WO | INSTRUCTION)  // Cache Prefetch Prediction Restriction by Context
    ARM32_CP_REG_DEFINE(DCCIMVAC,         15,   0,   7,  14,   1,   1, WO | INSTRUCTION | IGNORED)  // Data Cache line Clean and Invalidate by VA to PoC
    ARM32_CP_REG_DEFINE(DCCISW,           15,   0,   7,  14,   2,   1, WO | INSTRUCTION | IGNORED)  // Data Cache line Clean and Invalidate by Set/Way
    ARM32_CP_REG_DEFINE(DCCMVAC,          15,   0,   7,  10,   1,   1, WO | INSTRUCTION | IGNORED)  // Data Cache line Clean by VA to PoC
    ARM32_CP_REG_DEFINE(DCCMVAU,          15,   0,   7,  11,   1,   1, WO | INSTRUCTION | IGNORED)  // Data Cache line Clean by VA to PoU
    ARM32_CP_REG_DEFINE(DCCSW,            15,   0,   7,  10,   2,   1, WO | INSTRUCTION | IGNORED)  // Data Cache line Clean by Set/W ay
    ARM32_CP_REG_DEFINE(DCIMVAC,          15,   0,   7,   6,   1,   1, WO | INSTRUCTION | IGNORED)  // Data Cache line Invalidate by VA to PoC
    ARM32_CP_REG_DEFINE(DCISW,            15,   0,   7,   6,   2,   1, WO | INSTRUCTION | IGNORED)  // Data Cache line Invalidate by Set/Way
    ARM32_CP_REG_DEFINE(DTLBIALL,         15,   0,   8,   6,   0,   1, WO | INSTRUCTION)  // Data TLB Invalidate All
    ARM32_CP_REG_DEFINE(DTLBIASID,        15,   0,   8,   6,   2,   1, WO | INSTRUCTION)  // Data TLB Invalidate by ASID match
    ARM32_CP_REG_DEFINE(DTLBIMVA,         15,   0,   8,   6,   1,   1, WO | INSTRUCTION)  // Data TLB Invalidate by VA
    ARM32_CP_REG_DEFINE(DVPRCTX,          15,   0,   7,   3,   5,   0, WO | INSTRUCTION)  // Data V alue Prediction Restriction by Context
    ARM32_CP_REG_DEFINE(ICIALLU,          15,   0,   7,   5,   0,   1, WO | INSTRUCTION)  // Instruction Cache Invalidate All to PoU
    ARM32_CP_REG_DEFINE(ICIALLUIS,        15,   0,   7,   1,   0,   1, WO | INSTRUCTION)  // Instruction Cache Invalidate All to PoU, Inner Shareable
    ARM32_CP_REG_DEFINE(ICIMVAU,          15,   0,   7,   5,   1,   1, WO | INSTRUCTION)  // Instruction Cache line Invalidate by VA to PoU AArch32 System Instructions
    ARM32_CP_REG_DEFINE(ITLBIALL,         15,   0,   8,   5,   0,   1, WO | INSTRUCTION)  // Instruction TLB Invalidate All
    ARM32_CP_REG_DEFINE(ITLBIASID,        15,   0,   8,   5,   2,   1, WO | INSTRUCTION)  // Instruction TLB Invalidate by ASID match
    ARM32_CP_REG_DEFINE(ITLBIMVA,         15,   0,   8,   5,   1,   1, WO | INSTRUCTION)  // Instruction TLB Invalidate by VA
    ARM32_CP_REG_DEFINE(TLBIALL,          15,   0,   8,   7,   0,   1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))  // TLB Invalidate All
    ARM32_CP_REG_DEFINE(TLBIALLH,         15,   4,   8,   7,   0,   2, WO | INSTRUCTION)  // TLB Invalidate All, Hyp mode
    ARM32_CP_REG_DEFINE(TLBIALLHIS,       15,   4,   8,   3,   0,   2, WO | INSTRUCTION)  // TLB Invalidate All, Hyp mode, Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIALLIS,        15,   0,   8,   3,   0,   1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))  // TLB Invalidate All, Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIALLNSNH,      15,   4,   8,   7,   4,   2, WO | INSTRUCTION)  // TLB Invalidate All, Non-Secure Non-Hyp
    ARM32_CP_REG_DEFINE(TLBIALLNSNHIS,    15,   4,   8,   3,   4,   2, WO | INSTRUCTION)  // TLB Invalidate All, Non-Secure Non-Hyp, Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIASID,         15,   0,   8,   7,   2,   1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))  // TLB Invalidate by ASID match
    ARM32_CP_REG_DEFINE(TLBIASIDIS,       15,   0,   8,   3,   2,   1, WO | INSTRUCTION)  // TLB Invalidate by ASID match, Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIIPAS2,        15,   4,   8,   4,   1,   2, WO | INSTRUCTION)  // TLB Invalidate by Intermediate Physical Address, Stage 2
    ARM32_CP_REG_DEFINE(TLBIIPAS2IS,      15,   4,   8,   0,   1,   2, WO | INSTRUCTION)  // TLB Invalidate by Intermediate Physical Address, Stage 2, Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIIPAS2L,       15,   4,   8,   4,   5,   2, WO | INSTRUCTION)  // TLB Invalidate by Intermediate Physical Address, Stage 2, Last level
    ARM32_CP_REG_DEFINE(TLBIIPAS2LIS,     15,   4,   8,   0,   5,   2, WO | INSTRUCTION)  // TLB Invalidate by Intermediate Physical Address, Stage 2, Last level, Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIMVA,          15,   0,   8,   7,   1,   1, WO | INSTRUCTION)  // TLB Invalidate by VA
    ARM32_CP_REG_DEFINE(TLBIMVAA,         15,   0,   8,   7,   3,   1, WO | INSTRUCTION)  // TLB Invalidate by VA, All ASID
    ARM32_CP_REG_DEFINE(TLBIMVAAIS,       15,   0,   8,   3,   3,   1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))  // TLB Invalidate by VA, All ASID , Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIMVAAL,        15,   0,   8,   7,   7,   1, WO | INSTRUCTION)  // TLB Invalidate by VA, All ASID , Last level
    ARM32_CP_REG_DEFINE(TLBIMVAALIS,      15,   0,   8,   3,   7,   1, WO | INSTRUCTION)  // TLB Invalidate by VA, All ASID , Last level, Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIMVAH,         15,   4,   8,   7,   1,   2, WO | INSTRUCTION)  // TLB Invalidate by VA, Hyp mode
    ARM32_CP_REG_DEFINE(TLBIMVAHIS,       15,   4,   8,   3,   1,   2, WO | INSTRUCTION)  // TLB Invalidate by VA, Hyp mode, Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIMVAIS,        15,   0,   8,   3,   1,   1, WO | INSTRUCTION)  // TLB Invalidate by VA, Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIMVAL,         15,   0,   8,   7,   5,   1, WO | INSTRUCTION)  // TLB Invalidate by VA, Last level
    ARM32_CP_REG_DEFINE(TLBIMVALH,        15,   4,   8,   7,   5,   2, WO | INSTRUCTION)  // TLB Invalidate by VA, Last level, Hyp mode
    ARM32_CP_REG_DEFINE(TLBIMVALHIS,      15,   4,   8,   3,   5,   2, WO | INSTRUCTION)  // TLB Invalidate by VA, Last level, Hyp mode, Inner Shareable
    ARM32_CP_REG_DEFINE(TLBIMVALIS,       15,   0,   8,   3,   5,   1, WO | INSTRUCTION)  // TLB Invalidate by VA, Last level, Inner Shareable
};

ARMCPRegInfo aarch64_registers[] = {
    // The params are:   name                   op0, op1, crn, crm, op2, el, extra_type, ...
    ARM64_CP_REG_DEFINE(CurrentEL,               3,   0,   4,   2,   2,  0, ARM_CP_CURRENTEL)
    ARM64_CP_REG_DEFINE(ACCDATA_EL1,             3,   0,  11,   0,   5,  1, RW)
    ARM64_CP_REG_DEFINE(ACTLR_EL1,               3,   0,   1,   0,   1,  1, RW)
    ARM64_CP_REG_DEFINE(ACTLR_EL2,               3,   4,   1,   0,   1,  2, RW)
    ARM64_CP_REG_DEFINE(ACTLR_EL3,               3,   6,   1,   0,   1,  3, RW)
    ARM64_CP_REG_DEFINE(AFSR0_EL1,               3,   0,   5,   1,   0,  1, RW)
    ARM64_CP_REG_DEFINE(AFSR0_EL12,              3,   5,   5,   1,   0,  2, RW)
    ARM64_CP_REG_DEFINE(AFSR0_EL2,               3,   4,   5,   1,   0,  2, RW)
    ARM64_CP_REG_DEFINE(AFSR0_EL3,               3,   6,   5,   1,   0,  3, RW)
    ARM64_CP_REG_DEFINE(AFSR1_EL1,               3,   0,   5,   1,   1,  1, RW)
    ARM64_CP_REG_DEFINE(AFSR1_EL12,              3,   5,   5,   1,   1,  2, RW)
    ARM64_CP_REG_DEFINE(AFSR1_EL2,               3,   4,   5,   1,   1,  2, RW)
    ARM64_CP_REG_DEFINE(AFSR1_EL3,               3,   6,   5,   1,   1,  3, RW)
    ARM64_CP_REG_DEFINE(AIDR_EL1,                3,   1,   0,   0,   7,  1, RO)
    ARM64_CP_REG_DEFINE(ALLINT,                  3,   0,   4,   3,   0,  1, RW, RW_FNS(allint))
    ARM64_CP_REG_DEFINE(AMAIR_EL1,               3,   0,  10,   3,   0,  1, RW)
    ARM64_CP_REG_DEFINE(AMAIR_EL12,              3,   5,  10,   3,   0,  2, RW)
    ARM64_CP_REG_DEFINE(AMAIR_EL2,               3,   4,  10,   3,   0,  2, RW)
    ARM64_CP_REG_DEFINE(AMAIR_EL3,               3,   6,  10,   3,   0,  3, RW)
    ARM64_CP_REG_DEFINE(AMCFGR_EL0,              3,   3,  13,   2,   1,  0, RO)
    ARM64_CP_REG_DEFINE(AMCG1IDR_EL0,            3,   3,  13,   2,   6,  0, RO)
    ARM64_CP_REG_DEFINE(AMCGCR_EL0,              3,   3,  13,   2,   2,  0, RO)
    ARM64_CP_REG_DEFINE(AMCNTENCLR0_EL0,         3,   3,  13,   2,   4,  0, RW)
    ARM64_CP_REG_DEFINE(AMCNTENCLR1_EL0,         3,   3,  13,   3,   0,  0, RW)
    ARM64_CP_REG_DEFINE(AMCNTENSET0_EL0,         3,   3,  13,   2,   5,  0, RW)
    ARM64_CP_REG_DEFINE(AMCNTENSET1_EL0,         3,   3,  13,   3,   1,  0, RW)
    ARM64_CP_REG_DEFINE(AMCR_EL0,                3,   3,  13,   2,   0,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR00_EL0,          3,   3,  13,   4,   0,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR01_EL0,          3,   3,  13,   4,   1,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR02_EL0,          3,   3,  13,   4,   2,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR03_EL0,          3,   3,  13,   4,   3,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR10_EL0,          3,   3,  13,  12,   0,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR11_EL0,          3,   3,  13,  12,   1,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR12_EL0,          3,   3,  13,  12,   2,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR13_EL0,          3,   3,  13,  12,   3,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR14_EL0,          3,   3,  13,  12,   4,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR15_EL0,          3,   3,  13,  12,   5,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR16_EL0,          3,   3,  13,  12,   6,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR17_EL0,          3,   3,  13,  12,   7,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR18_EL0,          3,   3,  13,  13,   0,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR19_EL0,          3,   3,  13,  13,   1,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR110_EL0,         3,   3,  13,  13,   2,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR111_EL0,         3,   3,  13,  13,   3,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR112_EL0,         3,   3,  13,  13,   4,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR113_EL0,         3,   3,  13,  13,   5,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR114_EL0,         3,   3,  13,  13,   6,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTR115_EL0,         3,   3,  13,  13,   7,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF00_EL2,       3,   4,  13,   8,   0,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF01_EL2,       3,   4,  13,   8,   1,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF02_EL2,       3,   4,  13,   8,   2,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF03_EL2,       3,   4,  13,   8,   3,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF04_EL2,       3,   4,  13,   8,   4,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF05_EL2,       3,   4,  13,   8,   5,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF06_EL2,       3,   4,  13,   8,   6,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF07_EL2,       3,   4,  13,   8,   7,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF08_EL2,       3,   4,  13,   9,   0,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF09_EL2,       3,   4,  13,   9,   1,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF010_EL2,      3,   4,  13,   9,   2,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF011_EL2,      3,   4,  13,   9,   3,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF012_EL2,      3,   4,  13,   9,   4,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF013_EL2,      3,   4,  13,   9,   5,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF014_EL2,      3,   4,  13,   9,   6,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF015_EL2,      3,   4,  13,   9,   7,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF10_EL2,       3,   4,  13,  10,   0,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF11_EL2,       3,   4,  13,  10,   1,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF12_EL2,       3,   4,  13,  10,   2,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF13_EL2,       3,   4,  13,  10,   3,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF14_EL2,       3,   4,  13,  10,   4,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF15_EL2,       3,   4,  13,  10,   5,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF16_EL2,       3,   4,  13,  10,   6,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF17_EL2,       3,   4,  13,  10,   7,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF18_EL2,       3,   4,  13,  11,   0,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF19_EL2,       3,   4,  13,  11,   1,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF110_EL2,      3,   4,  13,  11,   2,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF111_EL2,      3,   4,  13,  11,   3,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF112_EL2,      3,   4,  13,  11,   4,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF113_EL2,      3,   4,  13,  11,   5,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF114_EL2,      3,   4,  13,  11,   6,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVCNTVOFF115_EL2,      3,   4,  13,  11,   7,  2, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER00_EL0,         3,   3,  13,   6,   0,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER10_EL0,         3,   3,  13,  14,   0,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER11_EL0,         3,   3,  13,  14,   1,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER12_EL0,         3,   3,  13,  14,   2,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER13_EL0,         3,   3,  13,  14,   3,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER14_EL0,         3,   3,  13,  14,   4,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER15_EL0,         3,   3,  13,  14,   5,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER16_EL0,         3,   3,  13,  14,   6,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER17_EL0,         3,   3,  13,  14,   7,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER18_EL0,         3,   3,  13,  15,   0,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER19_EL0,         3,   3,  13,  15,   1,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER110_EL0,        3,   3,  13,  15,   2,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER111_EL0,        3,   3,  13,  15,   3,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER112_EL0,        3,   3,  13,  15,   4,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER113_EL0,        3,   3,  13,  15,   5,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER114_EL0,        3,   3,  13,  15,   6,  0, RW)
    ARM64_CP_REG_DEFINE(AMEVTYPER115_EL0,        3,   3,  13,  15,   7,  0, RW)
    ARM64_CP_REG_DEFINE(AMUSERENR_EL0,           3,   3,  13,   2,   3,  0, RW)
    ARM64_CP_REG_DEFINE(APDAKeyHi_EL1,           3,   0,   2,   2,   1,  1, RW, FIELD(keys.apda.hi))
    ARM64_CP_REG_DEFINE(APDAKeyLo_EL1,           3,   0,   2,   2,   0,  1, RW, FIELD(keys.apda.lo))
    ARM64_CP_REG_DEFINE(APDBKeyHi_EL1,           3,   0,   2,   2,   3,  1, RW, FIELD(keys.apdb.hi))
    ARM64_CP_REG_DEFINE(APDBKeyLo_EL1,           3,   0,   2,   2,   2,  1, RW, FIELD(keys.apdb.lo))
    ARM64_CP_REG_DEFINE(APGAKeyHi_EL1,           3,   0,   2,   3,   1,  1, RW, FIELD(keys.apga.hi))
    ARM64_CP_REG_DEFINE(APGAKeyLo_EL1,           3,   0,   2,   3,   0,  1, RW, FIELD(keys.apga.lo))
    ARM64_CP_REG_DEFINE(APIAKeyHi_EL1,           3,   0,   2,   1,   1,  1, RW, FIELD(keys.apia.hi))
    ARM64_CP_REG_DEFINE(APIAKeyLo_EL1,           3,   0,   2,   1,   0,  1, RW, FIELD(keys.apia.lo))
    ARM64_CP_REG_DEFINE(APIBKeyHi_EL1,           3,   0,   2,   1,   3,  1, RW, FIELD(keys.apib.hi))
    ARM64_CP_REG_DEFINE(APIBKeyLo_EL1,           3,   0,   2,   1,   2,  1, RW, FIELD(keys.apib.lo))
    ARM64_CP_REG_DEFINE(CCSIDR_EL1,              3,   1,   0,   0,   0,  1, RO, READFN(ccsidr_el1))
    ARM64_CP_REG_DEFINE(CCSIDR2_EL1,             3,   1,   0,   0,   2,  1, RO, READFN(ccsidr2_el1))
    ARM64_CP_REG_DEFINE(CLIDR_EL1,               3,   1,   0,   0,   1,  1, RO, READFN(clidr_el1))
    // TODO: Implement trap on access to CNT* registers
    // The configuration of traping depends on flags from CNTHCTL_EL2 and CNTKCTL_EL1 registers
    ARM64_CP_REG_DEFINE(CNTFRQ_EL0,              3,   3,  14,   0,   0,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHCTL_EL2,             3,   4,  14,   1,   0,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHP_CTL_EL2,           3,   4,  14,   2,   1,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHP_CVAL_EL2,          3,   4,  14,   2,   2,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHP_TVAL_EL2,          3,   4,  14,   2,   0,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHPS_CTL_EL2,          3,   4,  14,   5,   1,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHPS_CVAL_EL2,         3,   4,  14,   5,   2,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHPS_TVAL_EL2,         3,   4,  14,   5,   0,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHV_CTL_EL2,           3,   4,  14,   3,   1,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHV_CVAL_EL2,          3,   4,  14,   3,   2,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHV_TVAL_EL2,          3,   4,  14,   3,   0,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHVS_CTL_EL2,          3,   4,  14,   4,   1,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHVS_CVAL_EL2,         3,   4,  14,   4,   2,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTHVS_TVAL_EL2,         3,   4,  14,   4,   0,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTKCTL_EL1,             3,   0,  14,   1,   0,  1, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTKCTL_EL12,            3,   5,  14,   1,   0,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTP_CTL_EL0,            3,   3,  14,   2,   1,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTP_CTL_EL02,           3,   5,  14,   2,   1,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTP_CVAL_EL0,           3,   3,  14,   2,   2,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTP_CVAL_EL02,          3,   5,  14,   2,   2,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTP_TVAL_EL0,           3,   3,  14,   2,   0,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTP_TVAL_EL02,          3,   5,  14,   2,   0,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTPCT_EL0,              3,   3,  14,   0,   1,  0, RO | GTIMER, READFN(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTPCTSS_EL0,            3,   3,  14,   0,   5,  0, RO | GTIMER, READFN(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTPOFF_EL2,             3,   4,  14,   0,   6,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTPS_CTL_EL1,           3,   7,  14,   2,   1,  1, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTPS_CVAL_EL1,          3,   7,  14,   2,   2,  1, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTPS_TVAL_EL1,          3,   7,  14,   2,   0,  1, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTV_CTL_EL0,            3,   3,  14,   3,   1,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTV_CTL_EL02,           3,   5,  14,   3,   1,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTV_CVAL_EL0,           3,   3,  14,   3,   2,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTV_CVAL_EL02,          3,   5,  14,   3,   2,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTV_TVAL_EL0,           3,   3,  14,   3,   0,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTV_TVAL_EL02,          3,   5,  14,   3,   0,  0, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTVCT_EL0,              3,   3,  14,   0,   2,  0, RO | GTIMER, READFN(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTVCTSS_EL0,            3,   3,  14,   0,   6,  0, RO | GTIMER, READFN(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CNTVOFF_EL2,             3,   4,  14,   0,   3,  2, RW | GTIMER, RW_FNS(generic_timer_aarch64))
    ARM64_CP_REG_DEFINE(CONTEXTIDR_EL1,          3,   0,  13,   0,   1,  1, RW | ARM_CP_TLB_FLUSH, RW_FNS(contextidr_el1))
    ARM64_CP_REG_DEFINE(CONTEXTIDR_EL12,         3,   5,  13,   0,   1,  2, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.contextidr_el[1]))
    ARM64_CP_REG_DEFINE(CONTEXTIDR_EL2,          3,   4,  13,   0,   1,  2, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.contextidr_el[2]))
    ARM64_CP_REG_DEFINE(CPACR_EL1,               3,   0,   1,   0,   2,  1, RW, RW_FNS(cpacr_el1))
    ARM64_CP_REG_DEFINE(CPACR_EL12,              3,   5,   1,   0,   2,  2, RW, FIELD(cp15.cpacr_el1))
    ARM64_CP_REG_DEFINE(CPTR_EL2,                3,   4,   1,   1,   2,  2, RW, FIELD(cp15.cptr_el[2]))
    ARM64_CP_REG_DEFINE(CPTR_EL3,                3,   6,   1,   1,   2,  3, RW, FIELD(cp15.cptr_el[3]))
    ARM64_CP_REG_DEFINE(CSSELR_EL1,              3,   2,   0,   0,   0,  1, RW, FIELD(cp15.csselr_el[1]))
    ARM64_CP_REG_DEFINE(CTR_EL0,                 3,   3,   0,   0,   1,  0, RO, READFN(ctr_el0))
    ARM64_CP_REG_DEFINE(DACR32_EL2,              3,   4,   3,   0,   0,  2, RW, FIELD(cp15.dacr32_el2))
    ARM64_CP_REG_DEFINE(DAIF,                    3,   3,   4,   2,   1,  0, RW, FIELD(daif))
    ARM64_CP_REG_DEFINE(DBGAUTHSTATUS_EL1,       2,   0,   7,  14,   6,  1, RO)
    ARM64_CP_REG_DEFINE(DBGBCR0_EL1,             2,   0,   0,   0,   5,  1, RW, FIELD(cp15.dbgbcr[0]))
    ARM64_CP_REG_DEFINE(DBGBCR1_EL1,             2,   0,   0,   1,   5,  1, RW, FIELD(cp15.dbgbcr[1]))
    ARM64_CP_REG_DEFINE(DBGBCR2_EL1,             2,   0,   0,   2,   5,  1, RW, FIELD(cp15.dbgbcr[2]))
    ARM64_CP_REG_DEFINE(DBGBCR3_EL1,             2,   0,   0,   3,   5,  1, RW, FIELD(cp15.dbgbcr[3]))
    ARM64_CP_REG_DEFINE(DBGBCR4_EL1,             2,   0,   0,   4,   5,  1, RW, FIELD(cp15.dbgbcr[4]))
    ARM64_CP_REG_DEFINE(DBGBCR5_EL1,             2,   0,   0,   5,   5,  1, RW, FIELD(cp15.dbgbcr[5]))
    ARM64_CP_REG_DEFINE(DBGBCR6_EL1,             2,   0,   0,   6,   5,  1, RW, FIELD(cp15.dbgbcr[6]))
    ARM64_CP_REG_DEFINE(DBGBCR7_EL1,             2,   0,   0,   7,   5,  1, RW, FIELD(cp15.dbgbcr[7]))
    ARM64_CP_REG_DEFINE(DBGBCR8_EL1,             2,   0,   0,   8,   5,  1, RW, FIELD(cp15.dbgbcr[8]))
    ARM64_CP_REG_DEFINE(DBGBCR9_EL1,             2,   0,   0,   9,   5,  1, RW, FIELD(cp15.dbgbcr[9]))
    ARM64_CP_REG_DEFINE(DBGBCR10_EL1,            2,   0,   0,  10,   5,  1, RW, FIELD(cp15.dbgbcr[10]))
    ARM64_CP_REG_DEFINE(DBGBCR11_EL1,            2,   0,   0,  11,   5,  1, RW, FIELD(cp15.dbgbcr[11]))
    ARM64_CP_REG_DEFINE(DBGBCR12_EL1,            2,   0,   0,  12,   5,  1, RW, FIELD(cp15.dbgbcr[12]))
    ARM64_CP_REG_DEFINE(DBGBCR13_EL1,            2,   0,   0,  13,   5,  1, RW, FIELD(cp15.dbgbcr[13]))
    ARM64_CP_REG_DEFINE(DBGBCR14_EL1,            2,   0,   0,  14,   5,  1, RW, FIELD(cp15.dbgbcr[14]))
    ARM64_CP_REG_DEFINE(DBGBCR15_EL1,            2,   0,   0,  15,   5,  1, RW, FIELD(cp15.dbgbcr[15]))
    ARM64_CP_REG_DEFINE(DBGBVR0_EL1,             2,   0,   0,   0,   4,  1, RW, FIELD(cp15.dbgbvr[0]))
    ARM64_CP_REG_DEFINE(DBGBVR1_EL1,             2,   0,   0,   1,   4,  1, RW, FIELD(cp15.dbgbvr[1]))
    ARM64_CP_REG_DEFINE(DBGBVR2_EL1,             2,   0,   0,   2,   4,  1, RW, FIELD(cp15.dbgbvr[2]))
    ARM64_CP_REG_DEFINE(DBGBVR3_EL1,             2,   0,   0,   3,   4,  1, RW, FIELD(cp15.dbgbvr[3]))
    ARM64_CP_REG_DEFINE(DBGBVR4_EL1,             2,   0,   0,   4,   4,  1, RW, FIELD(cp15.dbgbvr[4]))
    ARM64_CP_REG_DEFINE(DBGBVR5_EL1,             2,   0,   0,   5,   4,  1, RW, FIELD(cp15.dbgbvr[5]))
    ARM64_CP_REG_DEFINE(DBGBVR6_EL1,             2,   0,   0,   6,   4,  1, RW, FIELD(cp15.dbgbvr[6]))
    ARM64_CP_REG_DEFINE(DBGBVR7_EL1,             2,   0,   0,   7,   4,  1, RW, FIELD(cp15.dbgbvr[7]))
    ARM64_CP_REG_DEFINE(DBGBVR8_EL1,             2,   0,   0,   8,   4,  1, RW, FIELD(cp15.dbgbvr[8]))
    ARM64_CP_REG_DEFINE(DBGBVR9_EL1,             2,   0,   0,   9,   4,  1, RW, FIELD(cp15.dbgbvr[9]))
    ARM64_CP_REG_DEFINE(DBGBVR10_EL1,            2,   0,   0,  10,   4,  1, RW, FIELD(cp15.dbgbvr[10]))
    ARM64_CP_REG_DEFINE(DBGBVR11_EL1,            2,   0,   0,  11,   4,  1, RW, FIELD(cp15.dbgbvr[11]))
    ARM64_CP_REG_DEFINE(DBGBVR12_EL1,            2,   0,   0,  12,   4,  1, RW, FIELD(cp15.dbgbvr[12]))
    ARM64_CP_REG_DEFINE(DBGBVR13_EL1,            2,   0,   0,  13,   4,  1, RW, FIELD(cp15.dbgbvr[13]))
    ARM64_CP_REG_DEFINE(DBGBVR14_EL1,            2,   0,   0,  14,   4,  1, RW, FIELD(cp15.dbgbvr[14]))
    ARM64_CP_REG_DEFINE(DBGBVR15_EL1,            2,   0,   0,  15,   4,  1, RW, FIELD(cp15.dbgbvr[15]))
    ARM64_CP_REG_DEFINE(DBGCLAIMCLR_EL1,         2,   0,   7,   9,   6,  1, RW)
    ARM64_CP_REG_DEFINE(DBGCLAIMSET_EL1,         2,   0,   7,   8,   6,  1, RW)
    // Both 'DBGDTRRX_EL0' (RO) and 'DBGDTRTX_EL0' (WO) use the same encoding apart from the read/write bit.
    // We can't have two registers with the same op0+op1+crn+crm+op2 value so let's combine their names.
    ARM64_CP_REG_DEFINE(DBGDTR_EL0,              2,   3,   0,   4,   0,  0, RW)
    ARM64_CP_REG_DEFINE(DBGDTR_RX_TX_EL0,        2,   3,   0,   5,   0,  0, RW)
    ARM64_CP_REG_DEFINE(DBGPRCR_EL1,             2,   0,   1,   4,   4,  1, RW)
    ARM64_CP_REG_DEFINE(DBGVCR32_EL2,            2,   4,   0,   7,   0,  2, RW)
    ARM64_CP_REG_DEFINE(DBGWCR0_EL1,             2,   0,   0,   0,   7,  1, RW, FIELD(cp15.dbgwcr[0]))
    ARM64_CP_REG_DEFINE(DBGWCR1_EL1,             2,   0,   0,   1,   7,  1, RW, FIELD(cp15.dbgwcr[1]))
    ARM64_CP_REG_DEFINE(DBGWCR2_EL1,             2,   0,   0,   2,   7,  1, RW, FIELD(cp15.dbgwcr[2]))
    ARM64_CP_REG_DEFINE(DBGWCR3_EL1,             2,   0,   0,   3,   7,  1, RW, FIELD(cp15.dbgwcr[3]))
    ARM64_CP_REG_DEFINE(DBGWCR4_EL1,             2,   0,   0,   4,   7,  1, RW, FIELD(cp15.dbgwcr[4]))
    ARM64_CP_REG_DEFINE(DBGWCR5_EL1,             2,   0,   0,   5,   7,  1, RW, FIELD(cp15.dbgwcr[5]))
    ARM64_CP_REG_DEFINE(DBGWCR6_EL1,             2,   0,   0,   6,   7,  1, RW, FIELD(cp15.dbgwcr[6]))
    ARM64_CP_REG_DEFINE(DBGWCR7_EL1,             2,   0,   0,   7,   7,  1, RW, FIELD(cp15.dbgwcr[7]))
    ARM64_CP_REG_DEFINE(DBGWCR8_EL1,             2,   0,   0,   8,   7,  1, RW, FIELD(cp15.dbgwcr[8]))
    ARM64_CP_REG_DEFINE(DBGWCR9_EL1,             2,   0,   0,   9,   7,  1, RW, FIELD(cp15.dbgwcr[9]))
    ARM64_CP_REG_DEFINE(DBGWCR10_EL1,            2,   0,   0,  10,   7,  1, RW, FIELD(cp15.dbgwcr[10]))
    ARM64_CP_REG_DEFINE(DBGWCR11_EL1,            2,   0,   0,  11,   7,  1, RW, FIELD(cp15.dbgwcr[11]))
    ARM64_CP_REG_DEFINE(DBGWCR12_EL1,            2,   0,   0,  12,   7,  1, RW, FIELD(cp15.dbgwcr[12]))
    ARM64_CP_REG_DEFINE(DBGWCR13_EL1,            2,   0,   0,  13,   7,  1, RW, FIELD(cp15.dbgwcr[13]))
    ARM64_CP_REG_DEFINE(DBGWCR14_EL1,            2,   0,   0,  14,   7,  1, RW, FIELD(cp15.dbgwcr[14]))
    ARM64_CP_REG_DEFINE(DBGWCR15_EL1,            2,   0,   0,  15,   7,  1, RW, FIELD(cp15.dbgwcr[15]))
    ARM64_CP_REG_DEFINE(DBGWVR0_EL1,             2,   0,   0,   0,   6,  1, RW, FIELD(cp15.dbgwvr[0]))
    ARM64_CP_REG_DEFINE(DBGWVR1_EL1,             2,   0,   0,   1,   6,  1, RW, FIELD(cp15.dbgwvr[1]))
    ARM64_CP_REG_DEFINE(DBGWVR2_EL1,             2,   0,   0,   2,   6,  1, RW, FIELD(cp15.dbgwvr[2]))
    ARM64_CP_REG_DEFINE(DBGWVR3_EL1,             2,   0,   0,   3,   6,  1, RW, FIELD(cp15.dbgwvr[3]))
    ARM64_CP_REG_DEFINE(DBGWVR4_EL1,             2,   0,   0,   4,   6,  1, RW, FIELD(cp15.dbgwvr[4]))
    ARM64_CP_REG_DEFINE(DBGWVR5_EL1,             2,   0,   0,   5,   6,  1, RW, FIELD(cp15.dbgwvr[5]))
    ARM64_CP_REG_DEFINE(DBGWVR6_EL1,             2,   0,   0,   6,   6,  1, RW, FIELD(cp15.dbgwvr[6]))
    ARM64_CP_REG_DEFINE(DBGWVR7_EL1,             2,   0,   0,   7,   6,  1, RW, FIELD(cp15.dbgwvr[7]))
    ARM64_CP_REG_DEFINE(DBGWVR8_EL1,             2,   0,   0,   8,   6,  1, RW, FIELD(cp15.dbgwvr[8]))
    ARM64_CP_REG_DEFINE(DBGWVR9_EL1,             2,   0,   0,   9,   6,  1, RW, FIELD(cp15.dbgwvr[9]))
    ARM64_CP_REG_DEFINE(DBGWVR10_EL1,            2,   0,   0,  10,   6,  1, RW, FIELD(cp15.dbgwvr[10]))
    ARM64_CP_REG_DEFINE(DBGWVR11_EL1,            2,   0,   0,  11,   6,  1, RW, FIELD(cp15.dbgwvr[11]))
    ARM64_CP_REG_DEFINE(DBGWVR12_EL1,            2,   0,   0,  12,   6,  1, RW, FIELD(cp15.dbgwvr[12]))
    ARM64_CP_REG_DEFINE(DBGWVR13_EL1,            2,   0,   0,  13,   6,  1, RW, FIELD(cp15.dbgwvr[13]))
    ARM64_CP_REG_DEFINE(DBGWVR14_EL1,            2,   0,   0,  14,   6,  1, RW, FIELD(cp15.dbgwvr[14]))
    ARM64_CP_REG_DEFINE(DBGWVR15_EL1,            2,   0,   0,  15,   6,  1, RW, FIELD(cp15.dbgwvr[15]))
    ARM64_CP_REG_DEFINE(DCZID_EL0,               3,   3,   0,   0,   7,  0, RO, READFN(dczid))
    ARM64_CP_REG_DEFINE(DISR_EL1,                3,   0,  12,   1,   1,  1, RW, FIELD(cp15.disr_el1))
    ARM64_CP_REG_DEFINE(DIT,                     3,   3,   4,   2,   5,  0, RW, RW_FNS(dit))
    ARM64_CP_REG_DEFINE(DLR_EL0,                 3,   3,   4,   5,   1,  0, RW)
    ARM64_CP_REG_DEFINE(DSPSR_EL0,               3,   3,   4,   5,   0,  0, RW)
    ARM64_CP_REG_DEFINE(ELR_EL1,                 3,   0,   4,   0,   1,  1, RW, RW_FNS(elr_el1))
    ARM64_CP_REG_DEFINE(ELR_EL12,                3,   5,   4,   0,   1,  2, RW, FIELD(elr_el[1]))
    ARM64_CP_REG_DEFINE(ELR_EL2,                 3,   4,   4,   0,   1,  2, RW, FIELD(elr_el[2]))
    ARM64_CP_REG_DEFINE(ELR_EL3,                 3,   6,   4,   0,   1,  3, RW, FIELD(elr_el[3]))
    ARM64_CP_REG_DEFINE(ERRIDR_EL1,              3,   0,   5,   3,   0,  1, RO)
    ARM64_CP_REG_DEFINE(ERRSELR_EL1,             3,   0,   5,   3,   1,  1, RW)
    ARM64_CP_REG_DEFINE(ERXADDR_EL1,             3,   0,   5,   4,   3,  1, RW)
    ARM64_CP_REG_DEFINE(ERXCTLR_EL1,             3,   0,   5,   4,   1,  1, RW)
    ARM64_CP_REG_DEFINE(ERXFR_EL1,               3,   0,   5,   4,   0,  1, RO)
    ARM64_CP_REG_DEFINE(ERXMISC0_EL1,            3,   0,   5,   5,   0,  1, RW)
    ARM64_CP_REG_DEFINE(ERXMISC1_EL1,            3,   0,   5,   5,   1,  1, RW)
    ARM64_CP_REG_DEFINE(ERXMISC2_EL1,            3,   0,   5,   5,   2,  1, RW)
    ARM64_CP_REG_DEFINE(ERXMISC3_EL1,            3,   0,   5,   5,   3,  1, RW)
    ARM64_CP_REG_DEFINE(ERXPFGCDN_EL1,           3,   0,   5,   4,   6,  1, RW)
    ARM64_CP_REG_DEFINE(ERXPFGCTL_EL1,           3,   0,   5,   4,   5,  1, RW)
    ARM64_CP_REG_DEFINE(ERXPFGF_EL1,             3,   0,   5,   4,   4,  1, RO)
    ARM64_CP_REG_DEFINE(ERXSTATUS_EL1,           3,   0,   5,   4,   2,  1, RW)
    ARM64_CP_REG_DEFINE(ESR_EL1,                 3,   0,   5,   2,   0,  1, RW, RW_FNS(esr_el1))
    ARM64_CP_REG_DEFINE(ESR_EL12,                3,   5,   5,   2,   0,  2, RW, FIELD(cp15.esr_el[1]))
    ARM64_CP_REG_DEFINE(ESR_EL2,                 3,   4,   5,   2,   0,  2, RW, FIELD(cp15.esr_el[2]))
    ARM64_CP_REG_DEFINE(ESR_EL3,                 3,   6,   5,   2,   0,  3, RW, FIELD(cp15.esr_el[3]))
    ARM64_CP_REG_DEFINE(FAR_EL1,                 3,   0,   6,   0,   0,  1, RW, RW_FNS(far_el1))
    ARM64_CP_REG_DEFINE(FAR_EL12,                3,   5,   6,   0,   0,  2, RW, FIELD(cp15.far_el[1]))
    ARM64_CP_REG_DEFINE(FAR_EL2,                 3,   4,   6,   0,   0,  2, RW, FIELD(cp15.far_el[2]))
    ARM64_CP_REG_DEFINE(FAR_EL3,                 3,   6,   6,   0,   0,  3, RW, FIELD(cp15.far_el[3]))
    ARM64_CP_REG_DEFINE(FPCR,                    3,   3,   4,   4,   0,  0, RW, RW_FNS(fpcr))
    ARM64_CP_REG_DEFINE(FPEXC32_EL2,             3,   4,   5,   3,   0,  2, RW)
    ARM64_CP_REG_DEFINE(FPSR,                    3,   3,   4,   4,   1,  0, RW, RW_FNS(fpsr))
    ARM64_CP_REG_DEFINE(GCR_EL1,                 3,   0,   1,   0,   6,  1, RW, FIELD(cp15.gcr_el1))
    // TODO: find out the correct value, possible values:
    // Log2 of the block size in words. The minimum supported size is 16B (value == 2) and the maximum is 256B (value == 6).
    ARM64_CP_REG_DEFINE(GMID_EL1,                3,   1,   0,   0,   4,  1, RO | CONST(0x6))
    ARM64_CP_REG_DEFINE(HACR_EL2,                3,   4,   1,   1,   7,  2, RW)
    ARM64_CP_REG_DEFINE(HAFGRTR_EL2,             3,   4,   3,   1,   6,  2, RW)
    ARM64_CP_REG_DEFINE(HCR_EL2,                 3,   4,   1,   1,   0,  2, RW, FIELD(cp15.hcr_el2))
    ARM64_CP_REG_DEFINE(HCRX_EL2,                3,   4,   1,   2,   2,  2, RW, FIELD(cp15.hcrx_el2))
    ARM64_CP_REG_DEFINE(HDFGRTR_EL2,             3,   4,   3,   1,   4,  2, RW)
    ARM64_CP_REG_DEFINE(HDFGWTR_EL2,             3,   4,   3,   1,   5,  2, RW)
    ARM64_CP_REG_DEFINE(HFGITR_EL2,              3,   4,   1,   1,   6,  2, RW)
    ARM64_CP_REG_DEFINE(HFGRTR_EL2,              3,   4,   1,   1,   4,  2, RW)
    ARM64_CP_REG_DEFINE(HFGWTR_EL2,              3,   4,   1,   1,   5,  2, RW)
    ARM64_CP_REG_DEFINE(HPFAR_EL2,               3,   4,   6,   0,   4,  2, RW, FIELD(cp15.hpfar_el2))
    ARM64_CP_REG_DEFINE(HSTR_EL2,                3,   4,   1,   1,   3,  2, RW, FIELD(cp15.hstr_el2))
    // TODO: Implement trap on access to ICC_* registers
    // The configuration of traping depends on flags from ICC_SRE_EL* registers
    //
    // The 'ICV_*' registers are accessed using their equivalent 'ICC_*' mnemonics depending on the HCR_EL2's FMO/IMO bits.
    ARM64_CP_REG_DEFINE(ICC_AP0R0_EL1,           3,   0,  12,   8,   4,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_AP0R1_EL1,           3,   0,  12,   8,   5,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_AP0R2_EL1,           3,   0,  12,   8,   6,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_AP0R3_EL1,           3,   0,  12,   8,   7,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_AP1R0_EL1,           3,   0,  12,   9,   0,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_AP1R1_EL1,           3,   0,  12,   9,   1,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_AP1R2_EL1,           3,   0,  12,   9,   2,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_AP1R3_EL1,           3,   0,  12,   9,   3,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_ASGI1R_EL1,          3,   0,  12,  11,   6,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_BPR0_EL1,            3,   0,  12,   8,   3,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_BPR1_EL1,            3,   0,  12,  12,   3,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_CTLR_EL1,            3,   0,  12,  12,   4,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_CTLR_EL3,            3,   6,  12,  12,   4,  3, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_DIR_EL1,             3,   0,  12,  11,   1,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_EOIR0_EL1,           3,   0,  12,   8,   1,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_EOIR1_EL1,           3,   0,  12,  12,   1,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_HPPIR0_EL1,          3,   0,  12,   8,   2,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_HPPIR1_EL1,          3,   0,  12,  12,   2,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_IAR0_EL1,            3,   0,  12,   8,   0,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_IAR1_EL1,            3,   0,  12,  12,   0,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_IGRPEN0_EL1,         3,   0,  12,  12,   6,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_IGRPEN1_EL1,         3,   0,  12,  12,   7,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_IGRPEN1_EL3,         3,   6,  12,  12,   7,  3, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_NMIAR1_EL1,          3,   0,  12,   9,   5,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_PMR_EL1,             3,   0,   4,   6,   0,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_RPR_EL1,             3,   0,  12,  11,   3,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_SGI0R_EL1,           3,   0,  12,  11,   7,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_SGI1R_EL1,           3,   0,  12,  11,   5,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_SRE_EL1,             3,   0,  12,  12,   5,  1, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_SRE_EL2,             3,   4,  12,   9,   5,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICC_SRE_EL3,             3,   6,  12,  12,   5,  3, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_AP0R0_EL2,           3,   4,  12,   8,   0,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_AP0R1_EL2,           3,   4,  12,   8,   1,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_AP0R2_EL2,           3,   4,  12,   8,   2,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_AP0R3_EL2,           3,   4,  12,   8,   3,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_AP1R0_EL2,           3,   4,  12,   9,   0,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_AP1R1_EL2,           3,   4,  12,   9,   1,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_AP1R2_EL2,           3,   4,  12,   9,   2,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_AP1R3_EL2,           3,   4,  12,   9,   3,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_EISR_EL2,            3,   4,  12,  11,   3,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_ELRSR_EL2,           3,   4,  12,  11,   5,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_HCR_EL2,             3,   4,  12,  11,   0,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR0_EL2,             3,   4,  12,  12,   0,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR1_EL2,             3,   4,  12,  12,   1,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR2_EL2,             3,   4,  12,  12,   2,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR3_EL2,             3,   4,  12,  12,   3,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR4_EL2,             3,   4,  12,  12,   4,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR5_EL2,             3,   4,  12,  12,   5,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR6_EL2,             3,   4,  12,  12,   6,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR7_EL2,             3,   4,  12,  12,   7,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR8_EL2,             3,   4,  12,  13,   0,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR9_EL2,             3,   4,  12,  13,   1,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR10_EL2,            3,   4,  12,  13,   2,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR11_EL2,            3,   4,  12,  13,   3,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR12_EL2,            3,   4,  12,  13,   4,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR13_EL2,            3,   4,  12,  13,   5,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR14_EL2,            3,   4,  12,  13,   6,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_LR15_EL2,            3,   4,  12,  13,   7,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_MISR_EL2,            3,   4,  12,  11,   2,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_VMCR_EL2,            3,   4,  12,  11,   7,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ICH_VTR_EL2,             3,   4,  12,  11,   1,  2, RW | GIC, RW_FNS(interrupt_cpu_interface))
    ARM64_CP_REG_DEFINE(ID_AA64AFR0_EL1,         3,   0,   0,   5,   4,  1, RO, READFN(id_aa64afr0_el1))
    ARM64_CP_REG_DEFINE(ID_AA64AFR1_EL1,         3,   0,   0,   5,   5,  1, RO, READFN(id_aa64afr1_el1))
    ARM64_CP_REG_DEFINE(ID_AA64DFR0_EL1,         3,   0,   0,   5,   0,  1, RO, READFN(id_aa64dfr0_el1))
    ARM64_CP_REG_DEFINE(ID_AA64DFR1_EL1,         3,   0,   0,   5,   1,  1, RO)
    ARM64_CP_REG_DEFINE(ID_AA64ISAR0_EL1,        3,   0,   0,   6,   0,  1, RO, READFN(id_aa64isar0_el1))
    ARM64_CP_REG_DEFINE(ID_AA64ISAR1_EL1,        3,   0,   0,   6,   1,  1, RO, READFN(id_aa64isar1_el1))
    // TODO: Unimplemented
    // Prior to the introduction of the features described by this register, this register was unnamed and reserved, RES0 from EL1, EL2, and EL3.
    ARM64_CP_REG_DEFINE(ID_AA64ISAR2_EL1,        3,   0,   0,   6,   2,  1, RO)
    ARM64_CP_REG_DEFINE(ID_AA64MMFR0_EL1,        3,   0,   0,   7,   0,  1, RO, READFN(id_aa64mmfr0_el1))
    ARM64_CP_REG_DEFINE(ID_AA64MMFR1_EL1,        3,   0,   0,   7,   1,  1, RO, READFN(id_aa64mmfr1_el1))
    ARM64_CP_REG_DEFINE(ID_AA64MMFR2_EL1,        3,   0,   0,   7,   2,  1, RO, READFN(id_aa64mmfr2_el1))
    ARM64_CP_REG_DEFINE(ID_AA64PFR0_EL1,         3,   0,   0,   4,   0,  1, RO, READFN(id_aa64pfr0_el1))
    ARM64_CP_REG_DEFINE(ID_AA64PFR1_EL1,         3,   0,   0,   4,   1,  1, RO, READFN(id_aa64pfr1_el1))
    ARM64_CP_REG_DEFINE(ID_AA64SMFR0_EL1,        3,   0,   0,   4,   5,  1, RO, READFN(id_aa64smfr0_el1))
    ARM64_CP_REG_DEFINE(ID_AA64ZFR0_EL1,         3,   0,   0,   4,   4,  1, RO, READFN(id_aa64zfr0_el1))
    ARM64_CP_REG_DEFINE(ID_AFR0_EL1,             3,   0,   0,   1,   3,  1, RO, READFN(id_afr0))
    ARM64_CP_REG_DEFINE(ID_DFR0_EL1,             3,   0,   0,   1,   2,  1, RO, READFN(id_dfr0))
    ARM64_CP_REG_DEFINE(ID_DFR1_EL1,             3,   0,   0,   3,   5,  1, RO, READFN(id_dfr1))
    ARM64_CP_REG_DEFINE(ID_ISAR0_EL1,            3,   0,   0,   2,   0,  1, RO, READFN(id_isar0))
    ARM64_CP_REG_DEFINE(ID_ISAR1_EL1,            3,   0,   0,   2,   1,  1, RO, READFN(id_isar1))
    ARM64_CP_REG_DEFINE(ID_ISAR2_EL1,            3,   0,   0,   2,   2,  1, RO, READFN(id_isar2))
    ARM64_CP_REG_DEFINE(ID_ISAR3_EL1,            3,   0,   0,   2,   3,  1, RO, READFN(id_isar3))
    ARM64_CP_REG_DEFINE(ID_ISAR4_EL1,            3,   0,   0,   2,   4,  1, RO, READFN(id_isar4))
    ARM64_CP_REG_DEFINE(ID_ISAR5_EL1,            3,   0,   0,   2,   5,  1, RO, READFN(id_isar5))
    ARM64_CP_REG_DEFINE(ID_ISAR6_EL1,            3,   0,   0,   2,   7,  1, RO, READFN(id_isar6))
    ARM64_CP_REG_DEFINE(ID_MMFR0_EL1,            3,   0,   0,   1,   4,  1, RO, READFN(id_mmfr0))
    ARM64_CP_REG_DEFINE(ID_MMFR1_EL1,            3,   0,   0,   1,   5,  1, RO, READFN(id_mmfr1))
    ARM64_CP_REG_DEFINE(ID_MMFR2_EL1,            3,   0,   0,   1,   6,  1, RO, READFN(id_mmfr2))
    ARM64_CP_REG_DEFINE(ID_MMFR3_EL1,            3,   0,   0,   1,   7,  1, RO, READFN(id_mmfr3))
    ARM64_CP_REG_DEFINE(ID_MMFR4_EL1,            3,   0,   0,   2,   6,  1, RO, READFN(id_mmfr4))
    ARM64_CP_REG_DEFINE(ID_MMFR5_EL1,            3,   0,   0,   3,   6,  1, RO, READFN(id_mmfr5))
    ARM64_CP_REG_DEFINE(ID_PFR0_EL1,             3,   0,   0,   1,   0,  1, RO, READFN(id_pfr0))
    ARM64_CP_REG_DEFINE(ID_PFR1_EL1,             3,   0,   0,   1,   1,  1, RO, READFN(id_pfr1))
    ARM64_CP_REG_DEFINE(ID_PFR2_EL1,             3,   0,   0,   3,   4,  1, RO, READFN(id_pfr2))
    ARM64_CP_REG_DEFINE(IFSR32_EL2,              3,   4,   5,   0,   1,  2, RW, FIELD(cp15.ifsr32_el2))
    ARM64_CP_REG_DEFINE(ISR_EL1,                 3,   0,  12,   1,   0,  1, RO)
    ARM64_CP_REG_DEFINE(LORC_EL1,                3,   0,  10,   4,   3,  1, RW)
    ARM64_CP_REG_DEFINE(LOREA_EL1,               3,   0,  10,   4,   1,  1, RW)
    ARM64_CP_REG_DEFINE(LORID_EL1,               3,   0,  10,   4,   7,  1, RO)
    ARM64_CP_REG_DEFINE(LORN_EL1,                3,   0,  10,   4,   2,  1, RW)
    ARM64_CP_REG_DEFINE(LORSA_EL1,               3,   0,  10,   4,   0,  1, RW)
    ARM64_CP_REG_DEFINE(MAIR_EL1,                3,   0,  10,   2,   0,  1, RW, RW_FNS(mair_el1))
    ARM64_CP_REG_DEFINE(MAIR_EL12,               3,   5,  10,   2,   0,  2, RW, FIELD(cp15.mair_el[1]))
    ARM64_CP_REG_DEFINE(MAIR_EL2,                3,   4,  10,   2,   0,  2, RW, FIELD(cp15.mair_el[2]))
    ARM64_CP_REG_DEFINE(MAIR_EL3,                3,   6,  10,   2,   0,  3, RW, FIELD(cp15.mair_el[3]))
    ARM64_CP_REG_DEFINE(MDCCINT_EL1,             2,   0,   0,   2,   0,  1, RW)
    ARM64_CP_REG_DEFINE(MDCCSR_EL0,              2,   3,   0,   1,   0,  0, RO)
    ARM64_CP_REG_DEFINE(MDCR_EL2,                3,   4,   1,   1,   1,  2, RW, FIELD(cp15.mdcr_el2))
    ARM64_CP_REG_DEFINE(MDCR_EL3,                3,   6,   1,   3,   1,  3, RW, FIELD(cp15.mdcr_el3))
    ARM64_CP_REG_DEFINE(MDRAR_EL1,               2,   0,   1,   0,   0,  1, RO)
    ARM64_CP_REG_DEFINE(MDSCR_EL1,               2,   0,   0,   2,   2,  1, RW, FIELD(cp15.mdscr_el1))
    ARM64_CP_REG_DEFINE(MIDR_EL1,                3,   0,   0,   0,   0,  1, RO, READFN(midr))
    ARM64_CP_REG_DEFINE(MPAM0_EL1,               3,   0,  10,   5,   1,  1, RW)
    ARM64_CP_REG_DEFINE(MPAM1_EL1,               3,   0,  10,   5,   0,  1, RW)
    ARM64_CP_REG_DEFINE(MPAM2_EL2,               3,   4,  10,   5,   0,  2, RW)
    ARM64_CP_REG_DEFINE(MPAM3_EL3,               3,   6,  10,   5,   0,  3, RW)
    ARM64_CP_REG_DEFINE(MPAMHCR_EL2,             3,   4,  10,   4,   0,  2, RW)
    ARM64_CP_REG_DEFINE(MPAMIDR_EL1,             3,   0,  10,   4,   4,  1, RW)
    ARM64_CP_REG_DEFINE(MPAMVPM0_EL2,            3,   4,  10,   6,   0,  2, RW)
    ARM64_CP_REG_DEFINE(MPAMVPM1_EL2,            3,   4,  10,   6,   1,  2, RW)
    ARM64_CP_REG_DEFINE(MPAMVPM2_EL2,            3,   4,  10,   6,   2,  2, RW)
    ARM64_CP_REG_DEFINE(MPAMVPM3_EL2,            3,   4,  10,   6,   3,  2, RW)
    ARM64_CP_REG_DEFINE(MPAMVPM4_EL2,            3,   4,  10,   6,   4,  2, RW)
    ARM64_CP_REG_DEFINE(MPAMVPM5_EL2,            3,   4,  10,   6,   5,  2, RW)
    ARM64_CP_REG_DEFINE(MPAMVPM6_EL2,            3,   4,  10,   6,   6,  2, RW)
    ARM64_CP_REG_DEFINE(MPAMVPM7_EL2,            3,   4,  10,   6,   7,  2, RW)
    ARM64_CP_REG_DEFINE(MPAMVPMV_EL2,            3,   4,  10,   4,   1,  2, RW)
    ARM64_CP_REG_DEFINE(MPIDR_EL1,               3,   0,   0,   0,   5,  1, RO, READFN(mpidr_el1))
    ARM64_CP_REG_DEFINE(MVFR0_EL1,               3,   0,   0,   3,   0,  1, RO, READFN(mvfr0_el1))
    ARM64_CP_REG_DEFINE(MVFR1_EL1,               3,   0,   0,   3,   1,  1, RO, READFN(mvfr1_el1))
    ARM64_CP_REG_DEFINE(MVFR2_EL1,               3,   0,   0,   3,   2,  1, RO, READFN(mvfr2_el1))
    ARM64_CP_REG_DEFINE(NZCV,                    3,   3,   4,   2,   0,  0, RW | ARM_CP_NZCV, RW_FNS(nzcv))
    ARM64_CP_REG_DEFINE(OSDLR_EL1,               2,   0,   1,   3,   4,  1, RW, FIELD(cp15.osdlr_el1))
    ARM64_CP_REG_DEFINE(OSDTRRX_EL1,             2,   0,   0,   0,   2,  1, RW)
    ARM64_CP_REG_DEFINE(OSDTRTX_EL1,             2,   0,   0,   3,   2,  1, RW)
    ARM64_CP_REG_DEFINE(OSECCR_EL1,              2,   0,   0,   6,   2,  1, RW)
    ARM64_CP_REG_DEFINE(OSLAR_EL1,               2,   0,   1,   0,   4,  1, WO)
    ARM64_CP_REG_DEFINE(OSLSR_EL1,               2,   0,   1,   1,   4,  1, RW, FIELD(cp15.oslsr_el1))
    ARM64_CP_REG_DEFINE(PAN,                     3,   0,   4,   2,   3,  1, RW, RW_FNS(pan))
    ARM64_CP_REG_DEFINE(PAR_EL1,                 3,   0,   7,   4,   0,  1, RW, FIELD(cp15.par_el[1]))
    ARM64_CP_REG_DEFINE(PMBIDR_EL1,              3,   0,   9,  10,   7,  1, RO)
    ARM64_CP_REG_DEFINE(PMBLIMITR_EL1,           3,   0,   9,  10,   0,  1, RW)
    ARM64_CP_REG_DEFINE(PMBPTR_EL1,              3,   0,   9,  10,   1,  1, RW)
    ARM64_CP_REG_DEFINE(PMBSR_EL1,               3,   0,   9,  10,   3,  1, RW)
    ARM64_CP_REG_DEFINE(PMCCFILTR_EL0,           3,   3,  14,  15,   7,  0, RW)
    ARM64_CP_REG_DEFINE(PMCCNTR_EL0,             3,   3,   9,  13,   0,  0, RW)
    ARM64_CP_REG_DEFINE(PMCEID0_EL0,             3,   3,   9,  12,   6,  0, RO)
    ARM64_CP_REG_DEFINE(PMCEID1_EL0,             3,   3,   9,  12,   7,  0, RO)
    ARM64_CP_REG_DEFINE(PMCNTENCLR_EL0,          3,   3,   9,  12,   2,  0, RW, FIELD(cp15.c9_pmcnten))
    ARM64_CP_REG_DEFINE(PMCNTENSET_EL0,          3,   3,   9,  12,   1,  0, RW, FIELD(cp15.c9_pmcnten))
    ARM64_CP_REG_DEFINE(PMCR_EL0,                3,   3,   9,  12,   0,  0, RW, FIELD(cp15.c9_pmcr))
    ARM64_CP_REG_DEFINE(PMEVCNTR0_EL0,           3,   3,  14,   8,   0,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR1_EL0,           3,   3,  14,   8,   1,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR2_EL0,           3,   3,  14,   8,   2,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR3_EL0,           3,   3,  14,   8,   3,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR4_EL0,           3,   3,  14,   8,   4,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR5_EL0,           3,   3,  14,   8,   5,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR6_EL0,           3,   3,  14,   8,   6,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR7_EL0,           3,   3,  14,   8,   7,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR8_EL0,           3,   3,  14,   9,   0,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR9_EL0,           3,   3,  14,   9,   1,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR10_EL0,          3,   3,  14,   9,   2,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR11_EL0,          3,   3,  14,   9,   3,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR12_EL0,          3,   3,  14,   9,   4,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR13_EL0,          3,   3,  14,   9,   5,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR14_EL0,          3,   3,  14,   9,   6,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR15_EL0,          3,   3,  14,   9,   7,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR16_EL0,          3,   3,  14,  10,   0,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR17_EL0,          3,   3,  14,  10,   1,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR18_EL0,          3,   3,  14,  10,   2,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR19_EL0,          3,   3,  14,  10,   3,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR20_EL0,          3,   3,  14,  10,   4,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR21_EL0,          3,   3,  14,  10,   5,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR22_EL0,          3,   3,  14,  10,   6,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR23_EL0,          3,   3,  14,  10,   7,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR24_EL0,          3,   3,  14,  11,   0,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR25_EL0,          3,   3,  14,  11,   1,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR26_EL0,          3,   3,  14,  11,   2,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR27_EL0,          3,   3,  14,  11,   3,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR28_EL0,          3,   3,  14,  11,   4,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR29_EL0,          3,   3,  14,  11,   5,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVCNTR30_EL0,          3,   3,  14,  11,   6,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER0_EL0,          3,   3,  14,  12,   0,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER1_EL0,          3,   3,  14,  12,   1,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER2_EL0,          3,   3,  14,  12,   2,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER3_EL0,          3,   3,  14,  12,   3,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER4_EL0,          3,   3,  14,  12,   4,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER5_EL0,          3,   3,  14,  12,   5,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER6_EL0,          3,   3,  14,  12,   6,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER7_EL0,          3,   3,  14,  12,   7,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER8_EL0,          3,   3,  14,  13,   0,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER9_EL0,          3,   3,  14,  13,   1,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER10_EL0,         3,   3,  14,  13,   2,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER11_EL0,         3,   3,  14,  13,   3,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER12_EL0,         3,   3,  14,  13,   4,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER13_EL0,         3,   3,  14,  13,   5,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER14_EL0,         3,   3,  14,  13,   6,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER15_EL0,         3,   3,  14,  13,   7,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER16_EL0,         3,   3,  14,  14,   0,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER17_EL0,         3,   3,  14,  14,   1,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER18_EL0,         3,   3,  14,  14,   2,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER19_EL0,         3,   3,  14,  14,   3,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER20_EL0,         3,   3,  14,  14,   4,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER21_EL0,         3,   3,  14,  14,   5,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER22_EL0,         3,   3,  14,  14,   6,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER23_EL0,         3,   3,  14,  14,   7,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER24_EL0,         3,   3,  14,  15,   0,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER25_EL0,         3,   3,  14,  15,   1,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER26_EL0,         3,   3,  14,  15,   2,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER27_EL0,         3,   3,  14,  15,   3,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER28_EL0,         3,   3,  14,  15,   4,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER29_EL0,         3,   3,  14,  15,   5,  0, RW)
    ARM64_CP_REG_DEFINE(PMEVTYPER30_EL0,         3,   3,  14,  15,   6,  0, RW)
    ARM64_CP_REG_DEFINE(PMINTENCLR_EL1,          3,   0,   9,  14,   2,  1, RW, FIELD(cp15.c9_pminten))
    ARM64_CP_REG_DEFINE(PMINTENSET_EL1,          3,   0,   9,  14,   1,  1, RW, FIELD(cp15.c9_pminten))
    ARM64_CP_REG_DEFINE(PMMIR_EL1,               3,   0,   9,  14,   6,  1, RO)
    ARM64_CP_REG_DEFINE(PMOVSCLR_EL0,            3,   3,   9,  12,   3,  0, RW, FIELD(cp15.c9_pmovsr))
    ARM64_CP_REG_DEFINE(PMOVSSET_EL0,            3,   3,   9,  14,   3,  0, RW, FIELD(cp15.c9_pmovsr))
    ARM64_CP_REG_DEFINE(PMSCR_EL1,               3,   0,   9,   9,   0,  1, RW)
    ARM64_CP_REG_DEFINE(PMSCR_EL12,              3,   5,   9,   9,   0,  2, RW)
    ARM64_CP_REG_DEFINE(PMSCR_EL2,               3,   4,   9,   9,   0,  2, RW)
    ARM64_CP_REG_DEFINE(PMSELR_EL0,              3,   3,   9,  12,   5,  0, RW, FIELD(cp15.c9_pmselr))
    ARM64_CP_REG_DEFINE(PMSEVFR_EL1,             3,   0,   9,   9,   5,  1, RW)
    ARM64_CP_REG_DEFINE(PMSFCR_EL1,              3,   0,   9,   9,   4,  1, RW)
    ARM64_CP_REG_DEFINE(PMSIDR_EL1,              3,   0,   9,   9,   7,  1, RO)
    ARM64_CP_REG_DEFINE(PMSIRR_EL1,              3,   0,   9,   9,   3,  1, RW)
    ARM64_CP_REG_DEFINE(PMSLATFR_EL1,            3,   0,   9,   9,   6,  1, RW)
    ARM64_CP_REG_DEFINE(PMSNEVFR_EL1,            3,   0,   9,   9,   1,  1, RW)
    ARM64_CP_REG_DEFINE(PMSWINC_EL0,             3,   3,   9,  12,   4,  0, WO)
    ARM64_CP_REG_DEFINE(PMUSERENR_EL0,           3,   3,   9,  14,   0,  0, RW, FIELD(cp15.c9_pmuserenr))
    ARM64_CP_REG_DEFINE(PMXEVCNTR_EL0,           3,   3,   9,  13,   2,  0, RW)
    ARM64_CP_REG_DEFINE(PMXEVTYPER_EL0,          3,   3,   9,  13,   1,  0, RW)
    ARM64_CP_REG_DEFINE(REVIDR_EL1,              3,   0,   0,   0,   6,  1, RO, READFN(revidr_el1))
    ARM64_CP_REG_DEFINE(RGSR_EL1,                3,   0,   1,   0,   5,  1, RW, FIELD(cp15.rgsr_el1))
    ARM64_CP_REG_DEFINE(RMR_EL1,                 3,   0,  12,   0,   2,  1, RW)
    ARM64_CP_REG_DEFINE(RMR_EL2,                 3,   4,  12,   0,   2,  2, RW)
    ARM64_CP_REG_DEFINE(RMR_EL3,                 3,   6,  12,   0,   2,  3, RW)
    ARM64_CP_REG_DEFINE(RNDR,                    3,   3,   2,   4,   0,  0, RO, ACCESSFN(rndr), READFN(rndr))
    ARM64_CP_REG_DEFINE(RNDRRS,                  3,   3,   2,   4,   1,  0, RO, ACCESSFN(rndr), READFN(rndr))
    // TODO: Only one of RVBAR_ELx should be present -- the one for the highest available EL.
    ARM64_CP_REG_DEFINE(RVBAR_EL1,               3,   0,  12,   0,   1,  1, RO, FIELD(cp15.rvbar))
    ARM64_CP_REG_DEFINE(RVBAR_EL2,               3,   4,  12,   0,   1,  2, RO, FIELD(cp15.rvbar))
    ARM64_CP_REG_DEFINE(RVBAR_EL3,               3,   6,  12,   0,   1,  3, RO, FIELD(cp15.rvbar))
    ARM64_CP_REG_DEFINE(SCR_EL3,                 3,   6,   1,   1,   0,  3, RW, FIELD(cp15.scr_el3))
    ARM64_CP_REG_DEFINE(SCTLR_EL1,               3,   0,   1,   0,   0,  1, RW | ARM_CP_TLB_FLUSH, RW_FNS(sctlr_el1))
    ARM64_CP_REG_DEFINE(SCTLR_EL12,              3,   5,   1,   0,   0,  2, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.sctlr_el[1]))
    ARM64_CP_REG_DEFINE(SCTLR_EL2,               3,   4,   1,   0,   0,  2, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.sctlr_el[2]))
    ARM64_CP_REG_DEFINE(SCTLR_EL3,               3,   6,   1,   0,   0,  3, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.sctlr_el[3]))
    ARM64_CP_REG_DEFINE(SCXTNUM_EL0,             3,   3,  13,   0,   7,  0, RW, FIELD(scxtnum_el[0]))
    ARM64_CP_REG_DEFINE(SCXTNUM_EL1,             3,   0,  13,   0,   7,  1, RW, RW_FNS(scxtnum_el1))
    ARM64_CP_REG_DEFINE(SCXTNUM_EL12,            3,   5,  13,   0,   7,  2, RW, FIELD(scxtnum_el[1]))
    ARM64_CP_REG_DEFINE(SCXTNUM_EL2,             3,   4,  13,   0,   7,  2, RW, FIELD(scxtnum_el[2]))
    ARM64_CP_REG_DEFINE(SCXTNUM_EL3,             3,   6,  13,   0,   7,  3, RW, FIELD(scxtnum_el[3]))
    ARM64_CP_REG_DEFINE(SDER32_EL2,              3,   4,   1,   3,   1,  2, RW, FIELD(cp15.sder))
    ARM64_CP_REG_DEFINE(SDER32_EL3,              3,   6,   1,   1,   1,  3, RW, FIELD(cp15.sder))
    ARM64_CP_REG_DEFINE(SP_EL0,                  3,   0,   4,   1,   0,  0, RW, FIELD(sp_el[0]))
    ARM64_CP_REG_DEFINE(SP_EL1,                  3,   4,   4,   1,   0,  1, RW, FIELD(sp_el[1]))
    ARM64_CP_REG_DEFINE(SP_EL2,                  3,   6,   4,   1,   0,  3, RW, FIELD(sp_el[2]))
    ARM64_CP_REG_DEFINE(SPSel,                   3,   0,   4,   2,   0,  1, RW, RW_FNS(spsel))
    ARM64_CP_REG_DEFINE(SPSR_EL1,                3,   0,   4,   0,   0,  1, RW, RW_FNS(spsr_el1))
    ARM64_CP_REG_DEFINE(SPSR_EL12,               3,   5,   4,   0,   0,  2, RW, FIELD(banked_spsr[SPSR_EL1]))
    ARM64_CP_REG_DEFINE(SPSR_EL2,                3,   4,   4,   0,   0,  2, RW, FIELD(banked_spsr[SPSR_EL2]))
    ARM64_CP_REG_DEFINE(SPSR_EL3,                3,   6,   4,   0,   0,  3, RW, FIELD(banked_spsr[SPSR_EL3]))
    ARM64_CP_REG_DEFINE(SPSR_abt,                3,   4,   4,   3,   1,  2, RW, FIELD(banked_spsr[SPSR_ABT]))
    ARM64_CP_REG_DEFINE(SPSR_fiq,                3,   4,   4,   3,   3,  2, RW, FIELD(banked_spsr[SPSR_FIQ]))
    ARM64_CP_REG_DEFINE(SPSR_irq,                3,   4,   4,   3,   0,  2, RW, FIELD(banked_spsr[SPSR_IRQ]))
    ARM64_CP_REG_DEFINE(SPSR_und,                3,   4,   4,   3,   2,  2, RW, FIELD(banked_spsr[SPSR_UND]))
    ARM64_CP_REG_DEFINE(SSBS,                    3,   3,   4,   2,   6,  0, RW, RW_FNS(ssbs))
    ARM64_CP_REG_DEFINE(TCO,                     3,   3,   4,   2,   7,  0, RW, RW_FNS(tco))
    ARM64_CP_REG_DEFINE(TCR_EL1,                 3,   0,   2,   0,   2,  1, RW, RW_FNS(tcr_el1))
    ARM64_CP_REG_DEFINE(TCR_EL12,                3,   5,   2,   0,   2,  2, RW, FIELD(cp15.tcr_el[1]))
    ARM64_CP_REG_DEFINE(TCR_EL2,                 3,   4,   2,   0,   2,  2, RW, FIELD(cp15.tcr_el[2]))
    ARM64_CP_REG_DEFINE(TCR_EL3,                 3,   6,   2,   0,   2,  3, RW, FIELD(cp15.tcr_el[3]))
    ARM64_CP_REG_DEFINE(TFSR_EL1,                3,   0,   5,   6,   0,  1, RW, RW_FNS(tfsr_el1))
    ARM64_CP_REG_DEFINE(TFSR_EL12,               3,   5,   5,   6,   0,  2, RW, FIELD(cp15.tfsr_el[1]))
    ARM64_CP_REG_DEFINE(TFSR_EL2,                3,   4,   5,   6,   0,  2, RW, FIELD(cp15.tfsr_el[2]))
    ARM64_CP_REG_DEFINE(TFSR_EL3,                3,   6,   5,   6,   0,  3, RW, FIELD(cp15.tfsr_el[3]))
    ARM64_CP_REG_DEFINE(TFSRE0_EL1,              3,   0,   5,   6,   1,  1, RW, FIELD(cp15.tfsr_el[0]))
    ARM64_CP_REG_DEFINE(TPIDR_EL0,               3,   3,  13,   0,   2,  0, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.tpidr_el[0]))
    ARM64_CP_REG_DEFINE(TPIDR_EL1,               3,   0,  13,   0,   4,  1, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.tpidr_el[1]))
    ARM64_CP_REG_DEFINE(TPIDR_EL2,               3,   4,  13,   0,   2,  2, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.tpidr_el[2]))
    ARM64_CP_REG_DEFINE(TPIDR_EL3,               3,   6,  13,   0,   2,  3, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.tpidr_el[3]))
    ARM64_CP_REG_DEFINE(TPIDRRO_EL0,             3,   3,  13,   0,   3,  0, RW, FIELD(cp15.tpidrro_el[0]))
    ARM64_CP_REG_DEFINE(TTBR0_EL1,               3,   0,   2,   0,   0,  1, RW | ARM_CP_TLB_FLUSH, RW_FNS(ttbr0_el1))
    ARM64_CP_REG_DEFINE(TTBR0_EL12,              3,   5,   2,   0,   0,  2, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.ttbr0_el[1]))
    ARM64_CP_REG_DEFINE(TTBR0_EL2,               3,   4,   2,   0,   0,  2, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.ttbr0_el[2]))
    ARM64_CP_REG_DEFINE(TTBR0_EL3,               3,   6,   2,   0,   0,  3, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.ttbr0_el[3]))
    ARM64_CP_REG_DEFINE(TTBR1_EL1,               3,   0,   2,   0,   1,  1, RW | ARM_CP_TLB_FLUSH, RW_FNS(ttbr1_el1))
    ARM64_CP_REG_DEFINE(TTBR1_EL12,              3,   5,   2,   0,   1,  2, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.ttbr1_el[1]))
    ARM64_CP_REG_DEFINE(TTBR1_EL2,               3,   4,   2,   0,   1,  2, RW | ARM_CP_TLB_FLUSH, FIELD(cp15.ttbr1_el[2]))
    ARM64_CP_REG_DEFINE(UAO,                     3,   0,   4,   2,   4,  1, RW, RW_FNS(uao))
    ARM64_CP_REG_DEFINE(VBAR_EL1,                3,   0,  12,   0,   0,  1, RW, RW_FNS(vbar_el1))
    ARM64_CP_REG_DEFINE(VBAR_EL12,               3,   5,  12,   0,   0,  2, RW, FIELD(cp15.vbar_el[1]))
    ARM64_CP_REG_DEFINE(VBAR_EL2,                3,   4,  12,   0,   0,  2, RW, FIELD(cp15.vbar_el[2]))
    ARM64_CP_REG_DEFINE(VBAR_EL3,                3,   6,  12,   0,   0,  3, RW, FIELD(cp15.vbar_el[3]))
    ARM64_CP_REG_DEFINE(VDISR_EL2,               3,   4,  12,   1,   1,  2, RW, FIELD(cp15.disr_el1))
    ARM64_CP_REG_DEFINE(VMPIDR_EL2,              3,   4,   0,   0,   5,  2, RW, FIELD(cp15.vmpidr_el2))
    ARM64_CP_REG_DEFINE(VNCR_EL2,                3,   4,   2,   2,   0,  2, RW)
    ARM64_CP_REG_DEFINE(VPIDR_EL2,               3,   4,   0,   0,   0,  2, RW, FIELD(cp15.vpidr_el2))
    ARM64_CP_REG_DEFINE(VSESR_EL2,               3,   4,   5,   2,   3,  2, RW, FIELD(cp15.vsesr_el2))
    ARM64_CP_REG_DEFINE(VSTCR_EL2,               3,   4,   2,   6,   2,  2, RW, FIELD(cp15.vstcr_el2))
    ARM64_CP_REG_DEFINE(VSTTBR_EL2,              3,   4,   2,   6,   0,  2, RW, FIELD(cp15.vsttbr_el2))
    ARM64_CP_REG_DEFINE(VTCR_EL2,                3,   4,   2,   1,   2,  2, RW, FIELD(cp15.vtcr_el2))
    ARM64_CP_REG_DEFINE(VTTBR_EL2,               3,   4,   2,   1,   0,  2, RW, FIELD(cp15.vttbr_el2))
    ARM64_CP_REG_DEFINE(ZCR_EL1,                 3,   0,   1,   2,   0,  1, RW, RW_FNS(zcr_el1))
    ARM64_CP_REG_DEFINE(ZCR_EL12,                3,   5,   1,   2,   0,  2, RW, FIELD(vfp.zcr_el[1]))
    ARM64_CP_REG_DEFINE(ZCR_EL2,                 3,   4,   1,   2,   0,  2, RW, FIELD(vfp.zcr_el[2]))
    ARM64_CP_REG_DEFINE(ZCR_EL3,                 3,   6,   1,   2,   0,  3, RW, FIELD(vfp.zcr_el[3]))
};
// clang-format on

// clang-format off
ARMCPRegInfo aarch64_instructions[] = {
    // The params are:   name                   op0, op1, crn, crm, op2, el, extra_type, ...
    ARM64_CP_REG_DEFINE(AT S12E0R,               1,   4,   7,   8,   6,  0, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S12E0W,               1,   4,   7,   8,   7,  0, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S12E1R,               1,   4,   7,   8,   4,  1, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S12E1W,               1,   4,   7,   8,   5,  1, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S1E0R,                1,   0,   7,   8,   2,  0, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S1E0W,                1,   0,   7,   8,   3,  0, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S1E1R,                1,   0,   7,   8,   0,  1, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S1E1RP,               1,   0,   7,   9,   0,  1, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S1E1W,                1,   0,   7,   8,   1,  1, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S1E1WP,               1,   0,   7,   9,   1,  1, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S1E2R,                1,   4,   7,   8,   0,  2, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S1E2W,                1,   4,   7,   8,   1,  2, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S1E3R,                1,   6,   7,   8,   0,  3, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(AT S1E3W,                1,   6,   7,   8,   1,  3, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(CFP RCTX,                1,   3,   7,   3,   4,  0, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(CPP RCTX,                1,   3,   7,   3,   7,  0, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(DC CGDSW,                1,   0,   7,  10,   6,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CGDVAC,               1,   3,   7,  10,   5,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CGDVADP,              1,   3,   7,  13,   5,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CGDVAP,               1,   3,   7,  12,   5,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CGSW,                 1,   0,   7,  10,   4,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CGVAC,                1,   3,   7,  10,   3,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CGVADP,               1,   3,   7,  13,   3,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CGVAP,                1,   3,   7,  12,   3,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CIGDSW,               1,   0,   7,  14,   6,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CIGDVAC,              1,   3,   7,  14,   5,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CIGSW,                1,   0,   7,  14,   4,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CIGVAC,               1,   3,   7,  14,   3,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CISW,                 1,   0,   7,  14,   2,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CIVAC,                1,   3,   7,  14,   1,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CSW,                  1,   0,   7,  10,   2,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CVAC,                 1,   3,   7,  10,   1,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CVADP,                1,   3,   7,  13,   1,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CVAP,                 1,   3,   7,  12,   1,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC CVAU,                 1,   3,   7,  11,   1,  0, WO | INSTRUCTION | IGNORED)
    // DC GVA, DC GZVA and DC ZVA are handled differently in 'handle_sys'.
    ARM64_CP_REG_DEFINE(DC GVA,                  1,   3,   7,   4,   3,  0, WO | INSTRUCTION | ARM_CP_DC_GVA)
    ARM64_CP_REG_DEFINE(DC GZVA,                 1,   3,   7,   4,   4,  0, WO | INSTRUCTION | ARM_CP_DC_GZVA)
    ARM64_CP_REG_DEFINE(DC IGDSW,                1,   0,   7,   6,   6,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC IGDVAC,               1,   0,   7,   6,   5,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC IGSW,                 1,   0,   7,   6,   4,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC IGVAC,                1,   0,   7,   6,   3,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC ISW,                  1,   0,   7,   6,   2,  1, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC IVAC,                 1,   0,   7,   6,   1,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(DC ZVA,                  1,   3,   7,   4,   1,  0, WO | INSTRUCTION | ARM_CP_DC_ZVA)
    ARM64_CP_REG_DEFINE(DVP RCTX,                1,   3,   7,   3,   5,  0, WO | INSTRUCTION)
    ARM64_CP_REG_DEFINE(IC IALLU,                1,   0,   7,   5,   0,  1, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(IC IALLUIS,              1,   0,   7,   1,   0,  0, WO | INSTRUCTION, WRITEFN(ic_ialluis))
    ARM64_CP_REG_DEFINE(IC IVAU,                 1,   3,   7,   5,   1,  0, WO | INSTRUCTION | IGNORED)
    ARM64_CP_REG_DEFINE(TLBI ALLE1,              1,   4,   8,   7,   4,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE1IS,            1,   4,   8,   3,   4,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE1ISNXS,         1,   4,   9,   3,   4,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE1NXS,           1,   4,   9,   7,   4,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE1OS,            1,   4,   8,   1,   4,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE1OSNXS,         1,   4,   9,   1,   4,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE2,              1,   4,   8,   7,   0,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE2IS,            1,   4,   8,   3,   0,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE2ISNXS,         1,   4,   9,   3,   0,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE2NXS,           1,   4,   9,   7,   0,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE2OS,            1,   4,   8,   1,   0,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE2OSNXS,         1,   4,   9,   1,   0,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE3,              1,   6,   8,   7,   0,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE3IS,            1,   6,   8,   3,   0,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE3ISNXS,         1,   6,   9,   3,   0,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE3NXS,           1,   6,   9,   7,   0,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE3OS,            1,   6,   8,   1,   0,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ALLE3OSNXS,         1,   6,   9,   1,   0,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ASIDE1,             1,   0,   8,   7,   2,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ASIDE1IS,           1,   0,   8,   3,   2,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ASIDE1ISNXS,        1,   0,   9,   3,   2,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ASIDE1NXS,          1,   0,   9,   7,   2,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ASIDE1OS,           1,   0,   8,   1,   2,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI ASIDE1OSNXS,        1,   0,   9,   1,   2,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI IPAS2E1,            1,   4,   8,   4,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI IPAS2E1IS,          1,   4,   8,   0,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI IPAS2E1ISNXS,       1,   4,   9,   0,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI IPAS2E1NXS,         1,   4,   9,   4,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI IPAS2E1OS,          1,   4,   8,   4,   0,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI IPAS2E1OSNXS,       1,   4,   9,   4,   0,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI IPAS2LE1,           1,   4,   8,   4,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI IPAS2LE1IS,         1,   4,   8,   0,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI IPAS2LE1ISNXS,      1,   4,   9,   0,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI IPAS2LE1NXS,        1,   4,   9,   4,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI IPAS2LE1OS,         1,   4,   8,   4,   4,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI IPAS2LE1OSNXS,      1,   4,   9,   4,   4,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RIPAS2E1,           1,   4,   8,   4,   2,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RIPAS2E1IS,         1,   4,   8,   0,   2,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RIPAS2E1ISNXS,      1,   4,   9,   0,   2,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RIPAS2E1NXS,        1,   4,   9,   4,   2,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RIPAS2E1OS,         1,   4,   8,   4,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RIPAS2E1OSNXS,      1,   4,   9,   4,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RIPAS2LE1,          1,   4,   8,   4,   6,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RIPAS2LE1IS,        1,   4,   8,   0,   6,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RIPAS2LE1ISNXS,     1,   4,   9,   0,   6,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RIPAS2LE1NXS,       1,   4,   9,   4,   6,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RIPAS2LE1OS,        1,   4,   8,   4,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RIPAS2LE1OSNXS,     1,   4,   9,   4,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAAE1,             1,   0,   8,   6,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAAE1IS,           1,   0,   8,   2,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAAE1ISNXS,        1,   0,   9,   2,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAAE1NXS,          1,   0,   9,   6,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAAE1OS,           1,   0,   8,   5,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAAE1OSNXS,        1,   0,   9,   5,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAALE1,            1,   0,   8,   6,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAALE1IS,          1,   0,   8,   2,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAALE1ISNXS,       1,   0,   9,   2,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAALE1NXS,         1,   0,   9,   6,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAALE1OS,          1,   0,   8,   5,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAALE1OSNXS,       1,   0,   9,   5,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE1,              1,   0,   8,   6,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE1IS,            1,   0,   8,   2,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE1ISNXS,         1,   0,   9,   2,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE1NXS,           1,   0,   9,   6,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE1OS,            1,   0,   8,   5,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE1OSNXS,         1,   0,   9,   5,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE2,              1,   4,   8,   6,   1,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE2IS,            1,   4,   8,   2,   1,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE2ISNXS,         1,   4,   9,   2,   1,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE2NXS,           1,   4,   9,   6,   1,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE3,              1,   6,   8,   6,   1,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE3IS,            1,   6,   8,   2,   1,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE3ISNXS,         1,   6,   9,   2,   1,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE3NXS,           1,   6,   9,   6,   1,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE3OS,            1,   6,   8,   5,   1,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVAE3OSNXS,         1,   6,   9,   5,   1,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE1,             1,   0,   8,   6,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE1IS,           1,   0,   8,   2,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE1ISNXS,        1,   0,   9,   2,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE1NXS,          1,   0,   9,   6,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE1OS,           1,   0,   8,   5,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE1OSNXS,        1,   0,   9,   5,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE2,             1,   4,   8,   6,   5,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE2IS,           1,   4,   8,   2,   5,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE2ISNXS,        1,   4,   9,   2,   5,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE2NXS,          1,   4,   9,   6,   5,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE2OS,           1,   4,   8,   5,   5,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE2OSNXS,        1,   4,   9,   5,   5,  2, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE3,             1,   6,   8,   6,   5,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE3IS,           1,   6,   8,   2,   5,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE3ISNXS,        1,   6,   9,   2,   5,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE3NXS,          1,   6,   9,   6,   5,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE3OS,           1,   6,   8,   5,   5,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI RVALE3OSNXS,        1,   6,   9,   5,   5,  3, WO | INSTRUCTION, WRITEFN(tlbi_flush_all))
    ARM64_CP_REG_DEFINE(TLBI VAAE1,              1,   0,   8,   7,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAAE1IS,            1,   0,   8,   3,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAAE1ISNXS,         1,   0,   9,   3,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAAE1NXS,           1,   0,   9,   7,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAAE1OS,            1,   0,   8,   1,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAAE1OSNXS,         1,   0,   9,   1,   3,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAALE1,             1,   0,   8,   7,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAALE1IS,           1,   0,   8,   3,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAALE1ISNXS,        1,   0,   9,   3,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAALE1NXS,          1,   0,   9,   7,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAALE1OS,           1,   0,   8,   1,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAALE1OSNXS,        1,   0,   9,   1,   7,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE1,               1,   0,   8,   7,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE1IS,             1,   0,   8,   3,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE1ISNXS,          1,   0,   9,   3,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE1NXS,            1,   0,   9,   7,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE1OS,             1,   0,   8,   1,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE1OSNXS,          1,   0,   9,   1,   1,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE2,               1,   4,   8,   7,   1,  2, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE2IS,             1,   4,   8,   3,   1,  2, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE2ISNXS,          1,   4,   9,   3,   1,  2, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE2NXS,            1,   4,   9,   7,   1,  2, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE2OS,             1,   4,   8,   1,   1,  2, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE2OSNXS,          1,   4,   9,   1,   1,  2, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE3,               1,   6,   8,   7,   1,  3, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE3IS,             1,   6,   8,   3,   1,  3, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE3ISNXS,          1,   6,   9,   3,   1,  3, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE3NXS,            1,   6,   9,   7,   1,  3, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE3OS,             1,   6,   8,   1,   1,  3, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VAE3OSNXS,          1,   6,   9,   1,   1,  3, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE1,              1,   0,   8,   7,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE1IS,            1,   0,   8,   3,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE1ISNXS,         1,   0,   9,   3,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE1NXS,           1,   0,   9,   7,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE1OS,            1,   0,   8,   1,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE1OSNXS,         1,   0,   9,   1,   5,  1, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE2,              1,   4,   8,   7,   5,  2, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE2IS,            1,   4,   8,   3,   5,  2, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE2ISNXS,         1,   4,   9,   3,   5,  2, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE2NXS,           1,   4,   9,   7,   5,  2, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE2OS,            1,   4,   8,   1,   5,  2, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE2OSNXS,         1,   4,   9,   1,   5,  2, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE3,              1,   6,   8,   7,   5,  3, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE3IS,            1,   6,   8,   3,   5,  3, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE3ISNXS,         1,   6,   9,   3,   5,  3, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE3NXS,           1,   6,   9,   7,   5,  3, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE3OS,            1,   6,   8,   1,   5,  3, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VALE3OSNXS,         1,   6,   9,   1,   5,  3, WO | INSTRUCTION, WRITEFN(tlbi_va))
    ARM64_CP_REG_DEFINE(TLBI VMALLE1,            1,   0,   8,   7,   0,  1, WO | INSTRUCTION, WRITEFN(tlbi_vmall))
    ARM64_CP_REG_DEFINE(TLBI VMALLE1IS,          1,   0,   8,   3,   0,  1, WO | INSTRUCTION, WRITEFN(tlbi_vmall))
    ARM64_CP_REG_DEFINE(TLBI VMALLE1ISNXS,       1,   0,   9,   3,   0,  1, WO | INSTRUCTION, WRITEFN(tlbi_vmall))
    ARM64_CP_REG_DEFINE(TLBI VMALLE1NXS,         1,   0,   9,   7,   0,  1, WO | INSTRUCTION, WRITEFN(tlbi_vmall))
    ARM64_CP_REG_DEFINE(TLBI VMALLE1OS,          1,   0,   8,   1,   0,  1, WO | INSTRUCTION, WRITEFN(tlbi_vmall))
    ARM64_CP_REG_DEFINE(TLBI VMALLE1OSNXS,       1,   0,   9,   1,   0,  1, WO | INSTRUCTION, WRITEFN(tlbi_vmall))
    ARM64_CP_REG_DEFINE(TLBI VMALLS12E1,         1,   4,   8,   7,   6,  1, WO | INSTRUCTION, WRITEFN(tlbi_vmall))
    ARM64_CP_REG_DEFINE(TLBI VMALLS12E1IS,       1,   4,   8,   3,   6,  1, WO | INSTRUCTION, WRITEFN(tlbi_vmall))
    ARM64_CP_REG_DEFINE(TLBI VMALLS12E1ISNXS,    1,   4,   9,   3,   6,  1, WO | INSTRUCTION, WRITEFN(tlbi_vmall))
    ARM64_CP_REG_DEFINE(TLBI VMALLS12E1NXS,      1,   4,   9,   7,   6,  1, WO | INSTRUCTION, WRITEFN(tlbi_vmall))
    ARM64_CP_REG_DEFINE(TLBI VMALLS12E1OS,       1,   4,   8,   1,   6,  1, WO | INSTRUCTION, WRITEFN(tlbi_vmall))
    ARM64_CP_REG_DEFINE(TLBI VMALLS12E1OSNXS,    1,   4,   9,   1,   6,  1, WO | INSTRUCTION, WRITEFN(tlbi_vmall))
};
// clang-format on

void cp_reg_add(CPUState *env, ARMCPRegInfo *reg_info)
{
    uint32_t *key = tlib_malloc(sizeof(uint32_t));

    if(reg_info->type & ARM_CP_AARCH64) {
        *key = ENCODE_AA64_CP_REG(reg_info->cp, reg_info->crn, reg_info->crm, reg_info->op0, reg_info->op1, reg_info->op2);
    } else {
        bool ns = true;  //  TODO: Handle secure state banking in a correct way
        bool is64 = reg_info->type & ARM_CP_64BIT;
        *key = ENCODE_CP_REG(reg_info->cp, is64, ns, reg_info->crn, reg_info->crm, reg_info->op1, reg_info->op2);
    }

    cp_reg_add_with_key(env, env->cp_regs, key, reg_info);
}

//  TODO: TGAU: XXX_FUNCTION(32, ...) doesn't work
WRITE_FUNCTION(64, cp14_c6_teecr, {
    value &= 1;
    if(env->teecr != value) {
        env->teecr = value;
        tb_flush(env);
    }
})
READ_FUNCTION(64, cp14_c6_teecr, env->teecr)

/* Implementation defined registers.
 *
 * The 'op0' field is always 3 and 'crn' can only be either 11 or 15.
 */

// clang-format off
ARMCPRegInfo cortex_a9_regs[] =
{
    // The params are:  name                 cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(TEECR,               14,   6,   0,   0,   0,  1,  RW | ARM_CP_FORCE_TB_END, RW_FNS(cp14_c6_teecr)) // Thumb EE Configuration Register
    ARM32_CP_REG_DEFINE(TEEHBR,              14,   6,   1,   0,   0,  1,  RW, FIELD(teehbr))                               // Thumb EE Handler Base Register
    ARM32_CP_REG_DEFINE(PCR,                 15,   0,  15,   0,   0,  1,  RW | IGNORED)                                    // Power Control Register
    // TODO: TGAU: #93964 ARM implementation delegates this outside of tlib, but it resolves to 0xf8f00000 for Zynq-7000/Zedboard
    // It should also be RW.
    ARM32_CP_REG_DEFINE(CBAR,                15,   4,  15,   0,   0,  1,  RO, FIELD(cp15.c15_config_base_address), RESETVALUE(0xf8f00000))
};

ARMCPRegInfo cortex_a53_regs[] =
{
    // The params are:   name           op0, op1, crn, crm, op2, el, extra_type, ...
    ARM64_CP_REG_DEFINE(CBAR_EL1,        3,   1,  15,   3,   0,  1, RW)
    ARM64_CP_REG_DEFINE(CPUACTLR_EL1,    3,   1,  15,   2,   0,  1, RW)
    ARM64_CP_REG_DEFINE(CPUECTLR_EL1,    3,   1,  15,   2,   1,  1, RW)
    ARM64_CP_REG_DEFINE(CPUMERRSR_EL1,   3,   1,  15,   2,   2,  1, RW)
    ARM64_CP_REG_DEFINE(L2ACTLR_EL1,     3,   1,  15,   0,   0,  1, RW)
    ARM64_CP_REG_DEFINE(L2CTLR_EL1,      3,   1,  11,   0,   2,  1, RW)
    ARM64_CP_REG_DEFINE(L2ECTLR_EL1,     3,   1,  11,   0,   3,  1, RW)
    ARM64_CP_REG_DEFINE(L2MERRSR_EL1,    3,   1,  15,   2,   3,  1, RW)
};

ARMCPRegInfo cortex_a55_regs[] =
{
    // The params are:   name           op0, op1, crn, crm, op2, el, extra_type, ...
    ARM64_CP_REG_DEFINE(CLUSTERCFR_EL1,  3,   0,  15,   3,   0,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERIDR_EL1,  3,   0,  15,   3,   1,  1, RW)
    ARM64_CP_REG_DEFINE(CPUECTLR_EL1,    3,   0,  15,   1,   4,  1, RW)
};

ARMCPRegInfo cortex_a75_a76_a78_common_regs[] =
{
    // Beware that register summaries in the manual have the 'op0' parameter
    // named 'copro' and the 'op1'-'crn' order is reversed.
    //
    // The params are:   name                   op0, op1, crn, crm, op2, el, extra_type, ...
    ARM64_CP_REG_DEFINE(CPUACTLR_EL1,            3,   0,  15,   1,   0,  1, RW)
    ARM64_CP_REG_DEFINE(CPUACTLR2_EL1,           3,   0,  15,   1,   1,  1, RW)
    ARM64_CP_REG_DEFINE(CPUCFR_EL1,              3,   0,  15,   0,   0,  1, RO)
    ARM64_CP_REG_DEFINE(CPUECTLR_EL1,            3,   0,  15,   1,   4,  1, RW)
    ARM64_CP_REG_DEFINE(CPUPCR_EL3,              3,   6,  15,   8,   1,  3, RW)
    ARM64_CP_REG_DEFINE(CPUPMR_EL3,              3,   6,  15,   8,   3,  3, RW)
    ARM64_CP_REG_DEFINE(CPUPOR_EL3,              3,   6,  15,   8,   2,  3, RW)
    ARM64_CP_REG_DEFINE(CPUPSELR_EL3,            3,   6,  15,   8,   0,  3, RW)
    ARM64_CP_REG_DEFINE(CPUPWRCTLR_EL1,          3,   0,  15,   2,   7,  1, RW)
    ARM64_CP_REG_DEFINE(ERXPFGCDNR_EL1,          3,   0,  15,   2,   2,  1, RW)
    ARM64_CP_REG_DEFINE(ERXPFGCTLR_EL1,          3,   0,  15,   2,   1,  1, RW)
    ARM64_CP_REG_DEFINE(ERXPFGFR_EL1,            3,   0,  15,   2,   0,  1, RW)

    // Cluster registers
    ARM64_CP_REG_DEFINE(CLUSTERACPSID_EL1,       3,   0,  15,   4,   1,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERACTLR_EL1,        3,   0,  15,   3,   3,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERBUSQOS_EL1,       3,   0,  15,   4,   4,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERCFR_EL1,          3,   0,  15,   3,   0,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERECTLR_EL1,        3,   0,  15,   3,   4,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTEREVIDR_EL1,        3,   0,  15,   3,   2,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERIDR_EL1,          3,   0,  15,   3,   1,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERL3HIT_EL1,        3,   0,  15,   4,   5,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERL3MISS_EL1,       3,   0,  15,   4,   6,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPARTCR_EL1,       3,   0,  15,   4,   3,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMCEID0_EL1,      3,   0,  15,   6,   4,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMCEID1_EL1,      3,   0,  15,   6,   5,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMCLAIMCLR_EL1,   3,   0,  15,   6,   7,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMCLAIMSET_EL1,   3,   0,  15,   6,   6,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMCNTENCLR_EL1,   3,   0,  15,   5,   2,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMCNTENSET_EL1,   3,   0,  15,   5,   1,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMCR_EL1,         3,   0,  15,   5,   0,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMDBGCFG_EL1,     3,   0,  15,   6,   3,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMINTENCLR_EL1,   3,   0,  15,   5,   7,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMINTENSET_EL1,   3,   0,  15,   5,   6,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMOVSCLR_EL1,     3,   0,  15,   5,   4,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMOVSSET_EL1,     3,   0,  15,   5,   3,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMSELR_EL1,       3,   0,  15,   5,   5,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMXEVCNTR_EL1,    3,   0,  15,   6,   2,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPMXEVTYPER_EL1,   3,   0,  15,   6,   1,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPWRCTLR_EL1,      3,   0,  15,   3,   5,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPWRDN_EL1,        3,   0,  15,   3,   6,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERPWRSTAT_EL1,      3,   0,  15,   3,   7,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERSTASHSID_EL1,     3,   0,  15,   4,   2,  1, RW)
    ARM64_CP_REG_DEFINE(CLUSTERTHREADSID_EL1,    3,   0,  15,   4,   0,  1, RW)
};

// These are Cortex-A76 and Cortex-A78 registers that are an addition to the Cortex-A75 register set.
ARMCPRegInfo cortex_a76_a78_regs[] = {
    // Beware that register summaries in the manual have the 'op0' parameter
    // named 'copro' and the 'op1'-'crn' order is reversed.
    //
    // The params are:   name                   op0, op1, crn, crm, op2, el, extra_type, ...
    ARM64_CP_REG_DEFINE(ATCR_EL1,                3,   0,  15,   7,   0,  1, RW)
    ARM64_CP_REG_DEFINE(ATCR_EL12,               3,   5,  15,   7,   0,  2, RW)
    ARM64_CP_REG_DEFINE(ATCR_EL2,                3,   4,  15,   7,   0,  2, RW)
    ARM64_CP_REG_DEFINE(ATCR_EL3,                3,   6,  15,   7,   0,  3, RW)
    ARM64_CP_REG_DEFINE(AVTCR_EL2,               3,   4,  15,   7,   1,  2, RW)
    ARM64_CP_REG_DEFINE(CLUSTERTHREADSIDOVR_EL1, 3,   0,  15,   4,   7,  1, RW)
    ARM64_CP_REG_DEFINE(CPUACTLR3_EL1,           3,   0,  15,   1,   2,  1, RW)
};

// These are Cortex-A78 registers that are an addition to the Cortex-A76 register set.
ARMCPRegInfo cortex_a78_regs[] = {
    // Beware that register summaries in the manual have the 'op0' parameter
    // named 'copro' and the 'op1'-'crn' order is reversed.
    //
    // The params are:   name                   op0, op1, crn, crm, op2, el, extra_type, ...
    ARM64_CP_REG_DEFINE(CPUACTLR5_EL1,           3,   0,  15,   9,   0,  1, RW)
    ARM64_CP_REG_DEFINE(CPUACTLR6_EL1,           3,   0,  15,   9,   1,  1, RW)
    ARM64_CP_REG_DEFINE(CPUECTLR2_EL1,           3,   0,  15,   1,   5,  1, RW)
    ARM64_CP_REG_DEFINE(CPUPPMCR_EL3,            3,   6,  15,   2,   0,  3, RW)
};

ARMCPRegInfo cortex_r52_regs[] =
{
    // The params are:  name                 cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(IMP_ATCMREGIONR,     15,   0,   9,   1,   0,  1, RW, FIELD(cp15.tcm_region[0]), WRITEFN(tcm_region)) // TCM Region Register A
    ARM32_CP_REG_DEFINE(IMP_BTCMREGIONR,     15,   0,   9,   1,   1,  1, RW, FIELD(cp15.tcm_region[1]), WRITEFN(tcm_region)) // TCM Region Register B
    ARM32_CP_REG_DEFINE(IMP_CTCMREGIONR,     15,   0,   9,   1,   2,  1, RW, FIELD(cp15.tcm_region[2]), WRITEFN(tcm_region)) // TCM Region Register C
    ARM32_CP_REG_DEFINE(IMP_CSCTLR,          15,   1,   9,   1,   0,  1, RW) // Cache Segregation Control Register
    ARM32_CP_REG_DEFINE(IMP_BPCTLR,          15,   1,   9,   1,   1,  1, RW) // Branch Predictor Control Register
    ARM32_CP_REG_DEFINE(IMP_MEMPROTCLR,      15,   1,   9,   1,   2,  1, RW) // Memory Protection Control Register
    ARM32_CP_REG_DEFINE(IMP_SLAVEPCTLR,      15,   0,  11,   0,   0,  1, RW | CONST(0x1)) // Slave Port Control Register
    ARM32_CP_REG_DEFINE(IMP_PERIPHPREGIONR,  15,   0,  15,   0,   0,  1, RW) // Peripheral Port Region Register
    ARM32_CP_REG_DEFINE(IMP_FLASHIFREGIONR,  15,   0,  15,   0,   1,  1, RW) // Flash Interface Region Register
    ARM32_CP_REG_DEFINE(IMP_BUILDOPTR,       15,   0,  15,   2,   0,  1, RO) // Build Options Register
    ARM32_CP_REG_DEFINE(IMP_PINOPTR,         15,   0,  15,   2,   7,  1, RO) // Pin Options Register
    ARM32_CP_REG_DEFINE(IMP_DCIMALL,         15,   0,  15,   5,   0,  1, WO | INSTRUCTION | IGNORED)  // Data Cache line Invalidate All
    ARM32_CP_REG_DEFINE(IMP_CDBGDCI,         15,   0,  15,  15,   0,  1, WO | INSTRUCTION | IGNORED)  // Invalidate All Register
    ARM32_CP_REG_DEFINE(IMP_CBAR,            15,   1,  15,   3,   0,  1, RO) // Configuration Base Address Register
    ARM32_CP_REG_DEFINE(IMP_QOSR,            15,   1,  15,   3,   1,  1, RW) // Quality Of Service Register
    ARM32_CP_REG_DEFINE(IMP_BUSTIMEOUTR,     15,   1,  15,   3,   2,  1, RW) // Bus Timeout Register
    ARM32_CP_REG_DEFINE(IMP_INTMONR,         15,   1,  15,   3,   4,  1, RW) // Interrupt Monitoring Register
    ARM32_CP_REG_DEFINE(IMP_ICERR0,          15,   2,  15,   0,   0,  1, RW) // Instruction Cache Error Record Registers 0
    ARM32_CP_REG_DEFINE(IMP_ICERR1,          15,   2,  15,   0,   1,  1, RW) // Instruction Cache Error Record Registers 0
    ARM32_CP_REG_DEFINE(IMP_DCERR0,          15,   2,  15,   1,   0,  1, RW) // Data Cache Error Record Registers 0 and 1
    ARM32_CP_REG_DEFINE(IMP_DCERR1,          15,   2,  15,   1,   1,  1, RW) // Data Cache Error Record Registers 0 and 1
    ARM32_CP_REG_DEFINE(IMP_TCMERR0,         15,   2,  15,   2,   0,  1, RW) // TCM Error Record Register 0 and 1
    ARM32_CP_REG_DEFINE(IMP_TCMERR1,         15,   2,  15,   2,   1,  1, RW) // TCM Error Record Register 0 and 1
    ARM32_CP_REG_DEFINE(IMP_TCMSYNDR0,       15,   2,  15,   2,   2,  1, RO) // TCM Syndrome Register 0 and 1
    ARM32_CP_REG_DEFINE(IMP_TCMSYNDR1,       15,   2,  15,   2,   3,  1, RO) // TCM Syndrome Register 0 and 1
    ARM32_CP_REG_DEFINE(IMP_FLASHERR0,       15,   2,  15,   3,   0,  1, RW) // Flash Error Record Registers 0 and 1
    ARM32_CP_REG_DEFINE(IMP_FLASHERR1,       15,   2,  15,   3,   1,  1, RW) // Flash Error Record Registers 0 and 1
    ARM32_CP_REG_DEFINE(IMP_CDBGDR0,         15,   3,  15,   0,   0,  2, RO) // Cache Debug Data Register 0
    ARM32_CP_REG_DEFINE(IMP_CDBGDR1,         15,   3,  15,   0,   1,  2, RO) // Cache Debug Data Register 1
    ARM32_CP_REG_DEFINE(IMP_TESTR0,          15,   4,  15,   0,   0,  1, RO) // Test Register 0
    ARM32_CP_REG_DEFINE(IMP_TESTR1,          15,   4,  15,   0,   1,  1, WO) // This register is only for testing

    // The params are:        name           cp, op1, crm,  el, extra_type, ...
    ARM32_CP_64BIT_REG_DEFINE(CPUACTLR,      15,   0,  15,  1,  RW) // CPU Auxiliary Control Register
};

ARMCPRegInfo mpu_registers[] = {
    // The params are:  name                 cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(PRSELR,              15,   0,   6,   2,   1,  1, RW, FIELD(pmsav8.prselr)) // Protection Region Selection Register
    ARM32_CP_REG_DEFINE(PRBAR,               15,   0,   6,   3,   0,  1, RW, RW_FNS(prbar)) // Protection Region Base Address Register
    ARM32_CP_REG_DEFINE(PRLAR,               15,   0,   6,   3,   1,  1, RW, RW_FNS(prlar)) // Protection Region Limit Address Register
    ARM32_CP_REG_DEFINE(HPRBAR,              15,   4,   6,   3,   0,  2, RW, RW_FNS(hprbar)) // Hyp Protection Region Base Address Register
    ARM32_CP_REG_DEFINE(HPRLAR,              15,   4,   6,   3,   1,  2, RW, RW_FNS(hprlar)) // Hyp Protection Region Limit Address Register
    ARM32_CP_REG_DEFINE(HPRSELR,             15,   4,   6,   2,   1,  2, RW, FIELD(pmsav8.hprselr)) // Hyp Protection Region Selection Register
    ARM32_CP_REG_DEFINE(HPRENR,              15,   4,   6,   1,   1,  2, RW, RW_FNS(hprenr)) // Hyp MPU Region Enable Register
    ARM32_CP_REG_DEFINE(HMPUIR,              15,   4,   0,   0,   4,  2, RO, READFN(hmpuir)) // Hyp MPU Type Register
    ARM32_CP_REG_DEFINE(MPUIR,               15,   0,   0,   0,   4,  1, RO, READFN(mpuir)) // MPU Type Register

    // The params are:  name                 cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(PRBAR0,              15,   0,   6,   8,   0,  1, RW, RW_FNS(prbarn0)) // Protection Region Base Address Register 0
    ARM32_CP_REG_DEFINE(PRBAR1,              15,   0,   6,   8,   4,  1, RW, RW_FNS(prbarn1)) // Protection Region Base Address Register 1
    ARM32_CP_REG_DEFINE(PRBAR2,              15,   0,   6,   9,   0,  1, RW, RW_FNS(prbarn2)) // Protection Region Base Address Register 2
    ARM32_CP_REG_DEFINE(PRBAR3,              15,   0,   6,   9,   4,  1, RW, RW_FNS(prbarn3)) // Protection Region Base Address Register 3
    ARM32_CP_REG_DEFINE(PRBAR4,              15,   0,   6,  10,   0,  1, RW, RW_FNS(prbarn4)) // Protection Region Base Address Register 4
    ARM32_CP_REG_DEFINE(PRBAR5,              15,   0,   6,  10,   4,  1, RW, RW_FNS(prbarn5)) // Protection Region Base Address Register 5
    ARM32_CP_REG_DEFINE(PRBAR6,              15,   0,   6,  11,   0,  1, RW, RW_FNS(prbarn6)) // Protection Region Base Address Register 6
    ARM32_CP_REG_DEFINE(PRBAR7,              15,   0,   6,  11,   4,  1, RW, RW_FNS(prbarn7)) // Protection Region Base Address Register 7
    ARM32_CP_REG_DEFINE(PRBAR8,              15,   0,   6,  12,   0,  1, RW, RW_FNS(prbarn8)) // Protection Region Base Address Register 8
    ARM32_CP_REG_DEFINE(PRBAR9,              15,   0,   6,  12,   4,  1, RW, RW_FNS(prbarn9)) // Protection Region Base Address Register 9
    ARM32_CP_REG_DEFINE(PRBAR10,             15,   0,   6,  13,   0,  1, RW, RW_FNS(prbarn10)) // Protection Region Base Address Register 10
    ARM32_CP_REG_DEFINE(PRBAR11,             15,   0,   6,  13,   4,  1, RW, RW_FNS(prbarn11)) // Protection Region Base Address Register 11
    ARM32_CP_REG_DEFINE(PRBAR12,             15,   0,   6,  14,   0,  1, RW, RW_FNS(prbarn12)) // Protection Region Base Address Register 12
    ARM32_CP_REG_DEFINE(PRBAR13,             15,   0,   6,  14,   4,  1, RW, RW_FNS(prbarn13)) // Protection Region Base Address Register 13
    ARM32_CP_REG_DEFINE(PRBAR14,             15,   0,   6,  15,   0,  1, RW, RW_FNS(prbarn14)) // Protection Region Base Address Register 14
    ARM32_CP_REG_DEFINE(PRBAR15,             15,   0,   6,  15,   4,  1, RW, RW_FNS(prbarn15)) // Protection Region Base Address Register 15
    ARM32_CP_REG_DEFINE(PRBAR16,             15,   1,   6,   8,   0,  1, RW, RW_FNS(prbarn16)) // Protection Region Base Address Register 16
    ARM32_CP_REG_DEFINE(PRBAR17,             15,   1,   6,   8,   4,  1, RW, RW_FNS(prbarn17)) // Protection Region Base Address Register 17
    ARM32_CP_REG_DEFINE(PRBAR18,             15,   1,   6,   9,   0,  1, RW, RW_FNS(prbarn18)) // Protection Region Base Address Register 18
    ARM32_CP_REG_DEFINE(PRBAR19,             15,   1,   6,   9,   4,  1, RW, RW_FNS(prbarn19)) // Protection Region Base Address Register 19
    ARM32_CP_REG_DEFINE(PRBAR20,             15,   1,   6,  10,   0,  1, RW, RW_FNS(prbarn20)) // Protection Region Base Address Register 20
    ARM32_CP_REG_DEFINE(PRBAR21,             15,   1,   6,  10,   4,  1, RW, RW_FNS(prbarn21)) // Protection Region Base Address Register 21
    ARM32_CP_REG_DEFINE(PRBAR22,             15,   1,   6,  11,   0,  1, RW, RW_FNS(prbarn22)) // Protection Region Base Address Register 22
    ARM32_CP_REG_DEFINE(PRBAR23,             15,   1,   6,  11,   4,  1, RW, RW_FNS(prbarn23)) // Protection Region Base Address Register 23

    // The params are:  name                 cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(PRLAR0,              15,   0,   6,   8,   1,  1, RW, RW_FNS(prlarn0)) // Protection Region Limit Address Register 0
    ARM32_CP_REG_DEFINE(PRLAR1,              15,   0,   6,   8,   5,  1, RW, RW_FNS(prlarn1)) // Protection Region Limit Address Register 1
    ARM32_CP_REG_DEFINE(PRLAR2,              15,   0,   6,   9,   1,  1, RW, RW_FNS(prlarn2)) // Protection Region Limit Address Register 2
    ARM32_CP_REG_DEFINE(PRLAR3,              15,   0,   6,   9,   5,  1, RW, RW_FNS(prlarn3)) // Protection Region Limit Address Register 3
    ARM32_CP_REG_DEFINE(PRLAR4,              15,   0,   6,  10,   1,  1, RW, RW_FNS(prlarn4)) // Protection Region Limit Address Register 4
    ARM32_CP_REG_DEFINE(PRLAR5,              15,   0,   6,  10,   5,  1, RW, RW_FNS(prlarn5)) // Protection Region Limit Address Register 5
    ARM32_CP_REG_DEFINE(PRLAR6,              15,   0,   6,  11,   1,  1, RW, RW_FNS(prlarn6)) // Protection Region Limit Address Register 6
    ARM32_CP_REG_DEFINE(PRLAR7,              15,   0,   6,  11,   5,  1, RW, RW_FNS(prlarn7)) // Protection Region Limit Address Register 7
    ARM32_CP_REG_DEFINE(PRLAR8,              15,   0,   6,  12,   1,  1, RW, RW_FNS(prlarn8)) // Protection Region Limit Address Register 8
    ARM32_CP_REG_DEFINE(PRLAR9,              15,   0,   6,  12,   5,  1, RW, RW_FNS(prlarn9)) // Protection Region Limit Address Register 9
    ARM32_CP_REG_DEFINE(PRLAR10,             15,   0,   6,  13,   1,  1, RW, RW_FNS(prlarn10)) // Protection Region Limit Address Register 10
    ARM32_CP_REG_DEFINE(PRLAR11,             15,   0,   6,  13,   5,  1, RW, RW_FNS(prlarn11)) // Protection Region Limit Address Register 11
    ARM32_CP_REG_DEFINE(PRLAR12,             15,   0,   6,  14,   1,  1, RW, RW_FNS(prlarn12)) // Protection Region Limit Address Register 12
    ARM32_CP_REG_DEFINE(PRLAR13,             15,   0,   6,  14,   5,  1, RW, RW_FNS(prlarn13)) // Protection Region Limit Address Register 13
    ARM32_CP_REG_DEFINE(PRLAR14,             15,   0,   6,  15,   1,  1, RW, RW_FNS(prlarn14)) // Protection Region Limit Address Register 14
    ARM32_CP_REG_DEFINE(PRLAR15,             15,   0,   6,  15,   5,  1, RW, RW_FNS(prlarn15)) // Protection Region Limit Address Register 15
    ARM32_CP_REG_DEFINE(PRLAR16,             15,   1,   6,   8,   1,  1, RW, RW_FNS(prlarn16)) // Protection Region Limit Address Register 16
    ARM32_CP_REG_DEFINE(PRLAR17,             15,   1,   6,   8,   5,  1, RW, RW_FNS(prlarn17)) // Protection Region Limit Address Register 17
    ARM32_CP_REG_DEFINE(PRLAR18,             15,   1,   6,   9,   1,  1, RW, RW_FNS(prlarn18)) // Protection Region Limit Address Register 18
    ARM32_CP_REG_DEFINE(PRLAR19,             15,   1,   6,   9,   5,  1, RW, RW_FNS(prlarn19)) // Protection Region Limit Address Register 19
    ARM32_CP_REG_DEFINE(PRLAR20,             15,   1,   6,  10,   1,  1, RW, RW_FNS(prlarn20)) // Protection Region Limit Address Register 20
    ARM32_CP_REG_DEFINE(PRLAR21,             15,   1,   6,  10,   5,  1, RW, RW_FNS(prlarn21)) // Protection Region Limit Address Register 21
    ARM32_CP_REG_DEFINE(PRLAR22,             15,   1,   6,  11,   1,  1, RW, RW_FNS(prlarn22)) // Protection Region Limit Address Register 22
    ARM32_CP_REG_DEFINE(PRLAR23,             15,   1,   6,  11,   5,  1, RW, RW_FNS(prlarn23)) // Protection Region Limit Address Register 23

    // The params are:  name                 cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(HPRBAR0,             15,   4,   6,   8,   0,  2, RW, RW_FNS(hprbarn0)) // Hyp Protection Region Base Address Register 0
    ARM32_CP_REG_DEFINE(HPRBAR1,             15,   4,   6,   8,   4,  2, RW, RW_FNS(hprbarn1)) // Hyp Protection Region Base Address Register 1
    ARM32_CP_REG_DEFINE(HPRBAR2,             15,   4,   6,   9,   0,  2, RW, RW_FNS(hprbarn2)) // Hyp Protection Region Base Address Register 2
    ARM32_CP_REG_DEFINE(HPRBAR3,             15,   4,   6,   9,   4,  2, RW, RW_FNS(hprbarn3)) // Hyp Protection Region Base Address Register 3
    ARM32_CP_REG_DEFINE(HPRBAR4,             15,   4,   6,  10,   0,  2, RW, RW_FNS(hprbarn4)) // Hyp Protection Region Base Address Register 4
    ARM32_CP_REG_DEFINE(HPRBAR5,             15,   4,   6,  10,   4,  2, RW, RW_FNS(hprbarn5)) // Hyp Protection Region Base Address Register 5
    ARM32_CP_REG_DEFINE(HPRBAR6,             15,   4,   6,  11,   0,  2, RW, RW_FNS(hprbarn6)) // Hyp Protection Region Base Address Register 6
    ARM32_CP_REG_DEFINE(HPRBAR7,             15,   4,   6,  11,   4,  2, RW, RW_FNS(hprbarn7)) // Hyp Protection Region Base Address Register 7
    ARM32_CP_REG_DEFINE(HPRBAR8,             15,   4,   6,  12,   0,  2, RW, RW_FNS(hprbarn8)) // Hyp Protection Region Base Address Register 8
    ARM32_CP_REG_DEFINE(HPRBAR9,             15,   4,   6,  12,   4,  2, RW, RW_FNS(hprbarn9)) // Hyp Protection Region Base Address Register 9
    ARM32_CP_REG_DEFINE(HPRBAR10,            15,   4,   6,  13,   0,  2, RW, RW_FNS(hprbarn10)) // Hyp Protection Region Base Address Register 10
    ARM32_CP_REG_DEFINE(HPRBAR11,            15,   4,   6,  13,   4,  2, RW, RW_FNS(hprbarn11)) // Hyp Protection Region Base Address Register 11
    ARM32_CP_REG_DEFINE(HPRBAR12,            15,   4,   6,  14,   0,  2, RW, RW_FNS(hprbarn12)) // Hyp Protection Region Base Address Register 12
    ARM32_CP_REG_DEFINE(HPRBAR13,            15,   4,   6,  14,   4,  2, RW, RW_FNS(hprbarn13)) // Hyp Protection Region Base Address Register 13
    ARM32_CP_REG_DEFINE(HPRBAR14,            15,   4,   6,  15,   0,  2, RW, RW_FNS(hprbarn14)) // Hyp Protection Region Base Address Register 14
    ARM32_CP_REG_DEFINE(HPRBAR15,            15,   4,   6,  15,   4,  2, RW, RW_FNS(hprbarn15)) // Hyp Protection Region Base Address Register 15
    ARM32_CP_REG_DEFINE(HPRBAR16,            15,   5,   6,   8,   0,  2, RW, RW_FNS(hprbarn16)) // Hyp Protection Region Base Address Register 16
    ARM32_CP_REG_DEFINE(HPRBAR17,            15,   5,   6,   8,   4,  2, RW, RW_FNS(hprbarn17)) // Hyp Protection Region Base Address Register 17
    ARM32_CP_REG_DEFINE(HPRBAR18,            15,   5,   6,   9,   0,  2, RW, RW_FNS(hprbarn18)) // Hyp Protection Region Base Address Register 18
    ARM32_CP_REG_DEFINE(HPRBAR19,            15,   5,   6,   9,   4,  2, RW, RW_FNS(hprbarn19)) // Hyp Protection Region Base Address Register 19
    ARM32_CP_REG_DEFINE(HPRBAR20,            15,   5,   6,  10,   0,  2, RW, RW_FNS(hprbarn20)) // Hyp Protection Region Base Address Register 20
    ARM32_CP_REG_DEFINE(HPRBAR21,            15,   5,   6,  10,   4,  2, RW, RW_FNS(hprbarn21)) // Hyp Protection Region Base Address Register 21
    ARM32_CP_REG_DEFINE(HPRBAR22,            15,   5,   6,  11,   0,  2, RW, RW_FNS(hprbarn22)) // Hyp Protection Region Base Address Register 22
    ARM32_CP_REG_DEFINE(HPRBAR23,            15,   5,   6,  11,   4,  2, RW, RW_FNS(hprbarn23)) // Hyp Protection Region Base Address Register 23

    // The params are:  name                 cp, op1, crn, crm, op2, el, extra_type, ...
    ARM32_CP_REG_DEFINE(HPRLAR0,             15,   4,   6,   8,   1,  2, RW, RW_FNS(hprlarn0)) // Hyp Protection Region Limit Address Register 0
    ARM32_CP_REG_DEFINE(HPRLAR1,             15,   4,   6,   8,   5,  2, RW, RW_FNS(hprlarn1)) // Hyp Protection Region Limit Address Register 1
    ARM32_CP_REG_DEFINE(HPRLAR2,             15,   4,   6,   9,   1,  2, RW, RW_FNS(hprlarn2)) // Hyp Protection Region Limit Address Register 2
    ARM32_CP_REG_DEFINE(HPRLAR3,             15,   4,   6,   9,   5,  2, RW, RW_FNS(hprlarn3)) // Hyp Protection Region Limit Address Register 3
    ARM32_CP_REG_DEFINE(HPRLAR4,             15,   4,   6,  10,   1,  2, RW, RW_FNS(hprlarn4)) // Hyp Protection Region Limit Address Register 4
    ARM32_CP_REG_DEFINE(HPRLAR5,             15,   4,   6,  10,   5,  2, RW, RW_FNS(hprlarn5)) // Hyp Protection Region Limit Address Register 5
    ARM32_CP_REG_DEFINE(HPRLAR6,             15,   4,   6,  11,   1,  2, RW, RW_FNS(hprlarn6)) // Hyp Protection Region Limit Address Register 6
    ARM32_CP_REG_DEFINE(HPRLAR7,             15,   4,   6,  11,   5,  2, RW, RW_FNS(hprlarn7)) // Hyp Protection Region Limit Address Register 7
    ARM32_CP_REG_DEFINE(HPRLAR8,             15,   4,   6,  12,   1,  2, RW, RW_FNS(hprlarn8)) // Hyp Protection Region Limit Address Register 8
    ARM32_CP_REG_DEFINE(HPRLAR9,             15,   4,   6,  12,   5,  2, RW, RW_FNS(hprlarn9)) // Hyp Protection Region Limit Address Register 9
    ARM32_CP_REG_DEFINE(HPRLAR10,            15,   4,   6,  13,   1,  2, RW, RW_FNS(hprlarn10)) // Hyp Protection Region Limit Address Register 10
    ARM32_CP_REG_DEFINE(HPRLAR11,            15,   4,   6,  13,   5,  2, RW, RW_FNS(hprlarn11)) // Hyp Protection Region Limit Address Register 11
    ARM32_CP_REG_DEFINE(HPRLAR12,            15,   4,   6,  14,   1,  2, RW, RW_FNS(hprlarn12)) // Hyp Protection Region Limit Address Register 12
    ARM32_CP_REG_DEFINE(HPRLAR13,            15,   4,   6,  14,   5,  2, RW, RW_FNS(hprlarn13)) // Hyp Protection Region Limit Address Register 13
    ARM32_CP_REG_DEFINE(HPRLAR14,            15,   4,   6,  15,   1,  2, RW, RW_FNS(hprlarn14)) // Hyp Protection Region Limit Address Register 14
    ARM32_CP_REG_DEFINE(HPRLAR15,            15,   4,   6,  15,   5,  2, RW, RW_FNS(hprlarn15)) // Hyp Protection Region Limit Address Register 15
    ARM32_CP_REG_DEFINE(HPRLAR16,            15,   5,   6,   8,   1,  2, RW, RW_FNS(hprlarn16)) // Hyp Protection Region Limit Address Register 16
    ARM32_CP_REG_DEFINE(HPRLAR17,            15,   5,   6,   8,   5,  2, RW, RW_FNS(hprlarn17)) // Hyp Protection Region Limit Address Register 17
    ARM32_CP_REG_DEFINE(HPRLAR18,            15,   5,   6,   9,   1,  2, RW, RW_FNS(hprlarn18)) // Hyp Protection Region Limit Address Register 18
    ARM32_CP_REG_DEFINE(HPRLAR19,            15,   5,   6,   9,   5,  2, RW, RW_FNS(hprlarn19)) // Hyp Protection Region Limit Address Register 19
    ARM32_CP_REG_DEFINE(HPRLAR20,            15,   5,   6,  10,   1,  2, RW, RW_FNS(hprlarn20)) // Hyp Protection Region Limit Address Register 20
    ARM32_CP_REG_DEFINE(HPRLAR21,            15,   5,   6,  10,   5,  2, RW, RW_FNS(hprlarn21)) // Hyp Protection Region Limit Address Register 21
    ARM32_CP_REG_DEFINE(HPRLAR22,            15,   5,   6,  11,   1,  2, RW, RW_FNS(hprlarn22)) // Hyp Protection Region Limit Address Register 22
    ARM32_CP_REG_DEFINE(HPRLAR23,            15,   5,   6,  11,   5,  2, RW, RW_FNS(hprlarn23)) // Hyp Protection Region Limit Address Register 23
};
// clang-format on

void add_implementation_defined_registers(CPUState *env, uint32_t cpu_model_id)
{
    switch(cpu_model_id) {
        case ARM_CPUID_CORTEXA9:
            cp_regs_add(env, cortex_a9_regs, ARM_CP_ARRAY_COUNT(cortex_a9_regs));
            break;
        case ARM_CPUID_CORTEXA53:
            cp_regs_add(env, cortex_a53_regs, ARM_CP_ARRAY_COUNT(cortex_a53_regs));
            break;
        case ARM_CPUID_CORTEXA55:
            cp_regs_add(env, cortex_a55_regs, ARM_CP_ARRAY_COUNT(cortex_a55_regs));
            break;
        case ARM_CPUID_CORTEXA75:
            cp_regs_add(env, cortex_a75_a76_a78_common_regs, ARM_CP_ARRAY_COUNT(cortex_a75_a76_a78_common_regs));
            break;
        case ARM_CPUID_CORTEXA76:
            cp_regs_add(env, cortex_a75_a76_a78_common_regs, ARM_CP_ARRAY_COUNT(cortex_a75_a76_a78_common_regs));
            cp_regs_add(env, cortex_a76_a78_regs, ARM_CP_ARRAY_COUNT(cortex_a76_a78_regs));
            break;
        case ARM_CPUID_CORTEXA78:
            cp_regs_add(env, cortex_a75_a76_a78_common_regs, ARM_CP_ARRAY_COUNT(cortex_a75_a76_a78_common_regs));
            cp_regs_add(env, cortex_a76_a78_regs, ARM_CP_ARRAY_COUNT(cortex_a76_a78_regs));
            cp_regs_add(env, cortex_a78_regs, ARM_CP_ARRAY_COUNT(cortex_a78_regs));
            break;
        case ARM_CPUID_CORTEXR52:
            cp_regs_add(env, cortex_r52_regs, ARM_CP_ARRAY_COUNT(cortex_r52_regs));
            break;
        default:
            tlib_assert_not_reached();
    }
}

uint32_t get_implementation_defined_registers_count(uint32_t cpu_model_id)
{
    switch(cpu_model_id) {
        case ARM_CPUID_CORTEXA9:
            return ARM_CP_ARRAY_COUNT(cortex_a9_regs);
        case ARM_CPUID_CORTEXA53:
            return ARM_CP_ARRAY_COUNT(cortex_a53_regs);
        case ARM_CPUID_CORTEXA55:
            return ARM_CP_ARRAY_COUNT(cortex_a55_regs);
        case ARM_CPUID_CORTEXA75:
            return ARM_CP_ARRAY_COUNT(cortex_a75_a76_a78_common_regs);
        case ARM_CPUID_CORTEXA76:
            return ARM_CP_ARRAY_COUNT(cortex_a75_a76_a78_common_regs) + ARM_CP_ARRAY_COUNT(cortex_a76_a78_regs);
        case ARM_CPUID_CORTEXA78:
            return ARM_CP_ARRAY_COUNT(cortex_a75_a76_a78_common_regs) + ARM_CP_ARRAY_COUNT(cortex_a76_a78_regs) +
                   ARM_CP_ARRAY_COUNT(cortex_a78_regs);
        case ARM_CPUID_CORTEXR52:
            return ARM_CP_ARRAY_COUNT(cortex_r52_regs);
        default:
            tlib_assert_not_reached();
    }
}

//  The keys are dynamically allocated so let's make TTable free them when removing the entry.
void entry_remove_callback(TTable_entry *entry)
{
    tlib_free(entry->key);
}

void system_instructions_and_registers_init(CPUState *env, uint32_t cpu_model_id)
{
    uint32_t ttable_size = ARM_CP_ARRAY_COUNT(aarch32_instructions) + ARM_CP_ARRAY_COUNT(common_registers);
    ttable_size += get_implementation_defined_registers_count(cpu_model_id);
    if(arm_feature(env, ARM_FEATURE_PMSA)) {
        ttable_size += ARM_CP_ARRAY_COUNT(mpu_registers);
    }
    if(arm_feature(env, ARM_FEATURE_AARCH64)) {
        ttable_size += ARM_CP_ARRAY_COUNT(aarch64_instructions) + ARM_CP_ARRAY_COUNT(aarch64_registers);
    } else {
        ttable_size += ARM_CP_ARRAY_COUNT(aarch32_only_registers);
    }
    env->cp_regs = ttable_create(ttable_size, entry_remove_callback, ttable_compare_key_uint32);

    cp_regs_add(env, aarch32_instructions, ARM_CP_ARRAY_COUNT(aarch32_instructions));
    cp_regs_add(env, common_registers, ARM_CP_ARRAY_COUNT(common_registers));

    add_implementation_defined_registers(env, cpu_model_id);

    if(arm_feature(env, ARM_FEATURE_AARCH64)) {
        cp_regs_add(env, aarch64_instructions, ARM_CP_ARRAY_COUNT(aarch64_instructions));
        cp_regs_add(env, aarch64_registers, ARM_CP_ARRAY_COUNT(aarch64_registers));
    } else {
        cp_regs_add(env, aarch32_only_registers, ARM_CP_ARRAY_COUNT(aarch32_only_registers));
    }

    if(arm_feature(env, ARM_FEATURE_PMSA)) {
        cp_regs_add(env, mpu_registers, ARM_CP_ARRAY_COUNT(mpu_registers));
    }

    ttable_sort_by_keys(env->cp_regs);
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

        uint32_t width = ri->cp == CP_REG_ARM64_SYSREG_CP || (ri->type & ARM_CP_64BIT) ? 64 : 32;
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
