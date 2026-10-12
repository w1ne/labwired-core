/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

//  Required for target_ulong.
#include "cpu-defs.h"

#if defined(TARGET_ARM64)
#define MAX_TCM_REGIONS 3
#else
#define MAX_TCM_REGIONS 4
#endif

#define TCM_UNIT_SIZE                 0x200
#define TCM_MAX_SIZE                  (TCM_UNIT_SIZE << 0b01110)  //  Maximum configurable value
#define TCM_CONFIGURATION_FIELDS_MASK 0xFFF

#define TCM_CP15_OP2_DATA                   0
#define TCM_CP15_OP2_INSTRUCTION_OR_UNIFIED 1

#define TCM_SIZE_64KB 0b111

/* Check if the paramters meet the requirements from the spec. Otherwise abort core */
void validate_tcm_region(target_ulong base_address, target_ulong size, int region_index, target_ulong memory_granularity);
