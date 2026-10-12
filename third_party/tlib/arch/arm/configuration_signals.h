#pragma once

#include <stdbool.h>
#include <stdint.h>

//  That's currently because of using 'uint64_t included_signals_mask' in ConfigurationSignalsState.
#define CONFIGURATION_SIGNALS_MAX 64

//  Remember to update CONFIGURATION_SIGNALS_COUNT when modifying the enum. Don't assign any values.
#define CONFIGURATION_SIGNALS_COUNT 8
typedef enum {
    IN_DEBUG_ROM_ADDRESS,           //  DBGROMADDR
    IN_DEBUG_SELF_ADDRESS,          //  DBGSELFADDR
    IN_HIGH_EXCEPTION_VECTORS,      //  VINITHI
    IN_INITIALIZE_INSTRUCTION_TCM,  //  INITRAM
    IN_PERIPHERALS_BASE,            //  PERIPHBASE

    IN_AHB_REGION_REGISTER,          //  Based on PPHBASE, INITPPH and PPHSIZE
    IN_AXI_REGION_REGISTER,          //  Based on PPXBASE, INITPPX and PPXSIZE
    IN_VIRTUAL_AXI_REGION_REGISTER,  //  Based on PPVBASE, PPVSIZE
} ConfigurationSignals;

#if CONFIGURATION_SIGNALS_COUNT > CONFIGURATION_SIGNALS_MAX
#error Number of configuration signals is too large.
#endif

typedef struct CPUState CPUState;
void configuration_signals_apply(CPUState *env);

//  Will be gathered through a callback.
typedef struct {
    //  Each bit says whether to apply the signal's effect.
    //  Bit positions are based on the ConfigurationSignals enum.
    uint64_t included_signals_mask;

    uint32_t debug_rom_address;
    uint32_t debug_self_address;
    uint32_t high_exception_vectors;
    uint32_t initialize_instruction_tcm;
    uint32_t peripherals_base;

    uint32_t ahb_region_register;
    uint32_t axi_region_register;
    uint32_t virtual_axi_region_register;
} ConfigurationSignalsState;
