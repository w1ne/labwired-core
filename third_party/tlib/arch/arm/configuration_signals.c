/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#include <stdint.h>

#include "arch_callbacks.h"
#include "configuration_signals.h"
#include "cpu.h"
#include "tightly_coupled_memory.h"

static inline void handle_configuration_signal(CPUState *env, ConfigurationSignals signal_id, ConfigurationSignalsState state)
{
    uint32_t new_value, old_value;
    switch(signal_id) {
        case IN_DEBUG_ROM_ADDRESS:
            old_value = env->cp14.c1_dbgdrar;
            new_value = FIELD_DP32(old_value, DBGDRAR, ROMADDR, state.debug_rom_address);
            new_value = FIELD_DP32(new_value, DBGDRAR, Valid, DEBUG_ADDRESS_VALID_VALUE);
            env->cp14.c1_dbgdrar = new_value;
            break;
        case IN_DEBUG_SELF_ADDRESS: {
            //  The signal sets top 15 bits.
            uint32_t address_expanded_to_20bits = state.debug_self_address << 5;

            old_value = env->cp14.c2_dbgdsar;
            new_value = FIELD_DP32(old_value, DBGDSAR, SELFOFFSET, address_expanded_to_20bits);
            new_value = FIELD_DP32(new_value, DBGDSAR, Valid, DEBUG_ADDRESS_VALID_VALUE);
            env->cp14.c2_dbgdsar = new_value;
            break;
        }
        case IN_INITIALIZE_INSTRUCTION_TCM: {
            //  Cortex-R8 doesn't have any TCM selection register.
            int tcm_region = 0;
            int tcm_op2 = TCM_CP15_OP2_INSTRUCTION_OR_UNIFIED;

            uint32_t base_address = state.high_exception_vectors ? 0xFFFF0 : 0x0;
            //  TRM says the ITCM size is *maximum* 64KB if INITRAM and VINITHI are set.
            //  There's no default value but 0 seems invalid. Let's use 64KB.
            uint32_t size = TCM_SIZE_64KB;

            old_value = env->cp15.c9_tcmregion[tcm_op2][tcm_region];
            new_value = FIELD_DP32(old_value, ITCMRR, ENABLE_BIT, 1);
            new_value = FIELD_DP32(new_value, ITCMRR, SIZE, size);
            new_value = FIELD_DP32(new_value, ITCMRR, BASE_ADDRESS, base_address);
            env->cp15.c9_tcmregion[tcm_op2][tcm_region] = new_value;
            break;
        }
        case IN_HIGH_EXCEPTION_VECTORS:
            env->cp15.c1_sys = FIELD_DP32(env->cp15.c1_sys, SCTLR, V, state.high_exception_vectors);
            break;
        case IN_PERIPHERALS_BASE:
            env->cp15.c15_cbar = state.peripherals_base << 13;
            break;
        case IN_AHB_REGION_REGISTER:
            env->cp15.c15_ahb_region = state.ahb_region_register;
            break;
        case IN_AXI_REGION_REGISTER:
            env->cp15.c15_axi_region = state.axi_region_register;
            break;
        case IN_VIRTUAL_AXI_REGION_REGISTER:
            env->cp15.c15_virtual_axi_region = state.virtual_axi_region_register;
            break;
        default:
            tlib_abortf("Unknown configuration signal: %d", signal_id);
    }
}

static inline ConfigurationSignalsState get_state(void)
{
    ConfigurationSignalsState state = { 0 };
    tlib_fill_configuration_signals_state(&state);
    return state;
}

void configuration_signals_apply(CPUState *env)
{
    ConfigurationSignalsState state = get_state();

    uint32_t signal_id;
    for(signal_id = 0; signal_id < CONFIGURATION_SIGNALS_COUNT; signal_id++) {
        if(state.included_signals_mask & (UINT64_C(1) << signal_id)) {
            handle_configuration_signal(env, signal_id, state);
        }
    }
}
