/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#include <stdint.h>
#include "cpu.h"
#include "system_registers.h"
#include "tightly_coupled_memory.h"
#include "unwind.h"

uint32_t tlib_has_el3()
{
    return arm_feature(cpu, ARM_FEATURE_EL3);
}
EXC_INT_0(uint32_t, tlib_has_el3)

//  Clang, at least Apple clang version 13.0.0 (clang-1300.0.29.30; x86_64-apple-darwin20.5.0),
//  improperly optimizes the functions below (EXC_INT_1 generates a wrapper function).
//
//  If 'is_generic_timer_register(ri) == true' then the return value read through the wrapper
//  is correct but if 'is_gic_register(ri) == true' then the return value is invalid (false).
//
//  Disabling optimization for either of these functions fixes the issue.
//
//  The functions are also problematic with GCC 12+ when compiled with '-ftree-pre'.
//  A value read through the wrapper is always false even if any of 'is_gic_*' is true.
#ifdef __clang__
#pragma clang optimize off
#elif __GNUC__
//  Disable GCC's Partial Redundancy Elimination for this function.
__attribute__((optimize("-fno-tree-pre")))
#endif
bool tlib_is_gic_or_generic_timer_system_register(char *name)
{
    const ARMCPRegInfo *ri = sysreg_find_by_name(env, name);
    return ri != NULL && (is_gic_register(ri) || is_generic_timer_register(ri));
}
EXC_INT_1(bool, tlib_is_gic_or_generic_timer_system_register, char *, name)

#ifdef __clang__
#pragma clang optimize on
#endif

uint32_t tlib_set_available_els(bool el2_enabled, bool el3_enabled)
{
    enum {
        SIMULATION_ALREADY_STARTED = 1,
        SUCCESS = 3,
    };

    if(cpu->instructions_count_total_value != 0) {
        return SIMULATION_ALREADY_STARTED;
    }

    set_el_features(cpu, el2_enabled, el3_enabled);

    if(is_a64(env)) {
        //  Reset the Exception Level a CPU starts in.
        uint32_t reset_el = arm_highest_el(cpu);
        cpu->pstate = deposit32(cpu->pstate, 2, 2, reset_el);
    } else {
        uint32_t reset_mode = arm_get_highest_cpu_mode(env);
        cpsr_write(env, reset_mode, CPSR_M, CPSRWriteRaw);
    }

    arm_rebuild_hflags(cpu);

    tlib_on_execution_mode_changed(arm_current_el(env), arm_is_secure(env));

    return SUCCESS;
}
EXC_INT_2(uint32_t, tlib_set_available_els, bool, el2_enabled, bool, el3_enabled)

void tlib_psci_handler_enable(uint32_t mode)
{
    //  Options are mutually exclusive
    env->emulate_smc_calls = false;
    env->emulate_hvc_calls = false;

    switch(mode) {
        case 0:
            //  We have already disabled the handlers
            break;
        case 1:
            env->emulate_smc_calls = true;
            break;
        case 2:
            env->emulate_hvc_calls = true;
            break;
        default:
            tlib_abortf("Unknown PSCI handler mode %d", mode);
    }
}
EXC_VOID_1(tlib_psci_handler_enable, uint32_t, mode)

void tlib_set_current_el(uint32_t el)
{
    pstate_set_el(cpu, el);
}
EXC_VOID_1(tlib_set_current_el, uint32_t, el)

void tlib_set_mpu_regions_count(uint32_t el1_regions_count, uint32_t el2_regions_count)
{
    if(el1_regions_count > MAX_MPU_REGIONS || el2_regions_count > MAX_MPU_REGIONS) {
        tlib_abortf("Unable to set MPU regions count to %d. Maximum value for this core is %d",
                    el1_regions_count > el2_regions_count ? el1_regions_count : el2_regions_count, MAX_MPU_REGIONS);
    }

    set_pmsav8_regions_count(cpu, el1_regions_count, el2_regions_count);
}
EXC_VOID_2(tlib_set_mpu_regions_count, uint32_t, count, uint32_t, hyp_count)

/* Based on the documentation for Cortex-R52 */
void tlib_register_tcm_region(uint32_t address, uint64_t size, uint64_t region_index)
{
    if(size == 0) {
        //  Disable this region in the TCMTR
        cpu->cp15.tcm_type &= ~(1 << region_index);
        cpu->cp15.tcm_region[region_index] = 0;
    } else {
        validate_tcm_region(address, size, region_index, TARGET_PAGE_SIZE);

        //  Set this region as enabled
        cpu->cp15.tcm_type |= 1 << region_index;
        cpu->cp15.tcm_region[region_index] = address | (ctz64(size / TCM_UNIT_SIZE) << 2) | 3;  //  enable in all modes
    }

    //  Set TCMS bit - one or more TCMS implemented
    if(cpu->cp15.tcm_type & 0x7) {
        cpu->cp15.tcm_type |= 1u << 31;
    } else {
        cpu->cp15.tcm_type &= ~(1u << 31);
    }
}

EXC_VOID_3(tlib_register_tcm_region, uint32_t, address, uint64_t, size, uint64_t, index)

void tlib_enable_tcm_region(uint64_t region_index, uint32_t el01_enabled, uint32_t el2_enabled)
{
    bool flush_tlb = false;

    bool el01_enabled_current = cpu->cp15.tcm_region[region_index] & 1;
    if(el01_enabled_current != (el01_enabled > 0)) {
        cpu->cp15.tcm_region[region_index] = deposit64(cpu->cp15.tcm_region[region_index], 0, 1, el01_enabled > 0);
        flush_tlb = true;
    }

    bool el2_enabled_current = cpu->cp15.tcm_region[region_index] & 2;
    if(el2_enabled_current != (el2_enabled > 0)) {
        cpu->cp15.tcm_region[region_index] = deposit64(cpu->cp15.tcm_region[region_index], 1, 1, el2_enabled > 0);
        flush_tlb = true;
    }

    if(flush_tlb) {
        tlb_flush(env, 1, true);
    }
}

EXC_VOID_3(tlib_enable_tcm_region, uint64_t, region_index, uint32_t, el01_enabled, uint32_t, el2_enabled);

void tlib_set_gic_cpu_register_interface_version(uint32_t iface_version)
{
    env->arm_core_config.gic_cpu_interface_version = iface_version;
    env->arm_core_config.isar.id_aa64pfr0 = FIELD_DP64(env->arm_core_config.isar.id_aa64pfr0, ID_AA64PFR0, GIC, iface_version);
    env->arm_core_config.isar.id_pfr1 = FIELD_DP64(env->arm_core_config.isar.id_pfr1, ID_PFR1, GIC, iface_version);
}

EXC_VOID_1(tlib_set_gic_cpu_register_interface_version, uint32_t, iface_version)

void tlib_set_rndr_supported(uint32_t rndr)
{
    env->arm_core_config.isar.id_aa64isar0 = FIELD_DP64(env->arm_core_config.isar.id_aa64isar0, ID_AA64ISAR0, RNDR, rndr);
}

EXC_VOID_1(tlib_set_rndr_supported, uint32_t, rndr)
