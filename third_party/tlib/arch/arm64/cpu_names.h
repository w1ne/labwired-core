/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

#include <stddef.h>
#include <inttypes.h>

//  From the core's "Main ID Register", i.e., MIDR/MIDR_EL1.
//  ARMv7
#define ARM_CPUID_CORTEXA9 0x410fc090
//  ARMv8
#define ARM_CPUID_CORTEXA53 0x410fd034
#define ARM_CPUID_CORTEXA55 0x411fd050
#define ARM_CPUID_CORTEXA75 0x413fd0a1
#define ARM_CPUID_CORTEXA76 0x414fd0b1
#define ARM_CPUID_CORTEXA78 0x411fd412
#define ARM_CPUID_CORTEXR52 0x411fd132
#define ARM_CPUID_NOT_FOUND 0x0

static const struct arm_cpu_t arm_cpu_names[] = {
    { ARM_CPUID_CORTEXA9,  "cortex-a9"  },
    { ARM_CPUID_CORTEXA53, "cortex-a53" },
    { ARM_CPUID_CORTEXA55, "cortex-a55" },
    { ARM_CPUID_CORTEXA75, "cortex-a75" },
    { ARM_CPUID_CORTEXA76, "cortex-a76" },
    { ARM_CPUID_CORTEXA78, "cortex-a78" },
    { ARM_CPUID_CORTEXR52, "cortex-r52" },
    { ARM_CPUID_NOT_FOUND, NULL         },
};

uint32_t cpu_arm_find_by_name(const char *name);
