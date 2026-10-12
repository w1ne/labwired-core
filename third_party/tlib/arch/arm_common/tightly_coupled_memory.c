/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#include "bit_helper.h"
#include "cpu-defs.h"
#include "infrastructure.h"
#include "tightly_coupled_memory.h"

/* Check if the paramters meet the requirements from the spec. Otherwise abort core */
void validate_tcm_region(target_ulong base_address, target_ulong size, int region_index, target_ulong memory_granularity)
{
    if(region_index >= MAX_TCM_REGIONS) {
        tlib_abortf("Attempted to register TCM region #%u, maximal supported value is %u", region_index, MAX_TCM_REGIONS);
    }
    //  TODO: this should be configurable
    if(region_index >= 3) {
        tlib_abortf("Attempted to register TCM region  #%u. This core supports only %u", region_index, 3);
    }

    if((size < TCM_UNIT_SIZE) || size > TCM_MAX_SIZE || !is_power_of_2(size)) {
        tlib_abortf("Attempted to register TCM region #%u with incorrect size 0x%x", region_index, size);
    }

    if((base_address % memory_granularity) || (base_address & TCM_CONFIGURATION_FIELDS_MASK) || (base_address % size)) {
        tlib_abortf("Attempted to set illegal TCM region base address (0x%llx)", base_address);
    }
}
