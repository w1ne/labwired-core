/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

#include <stdint.h>
#include "cpu.h"
#include "system_registers_common.h"

//  This doesn't seem to be of any real use. It's passed to ENCODE_AA64_CP_REG in 'translate-a64.c:handle_sys'
//  as 'cp' so it seems to be a coprocessor ID. However, there's no information about coprocessor for AArch64
//  registers and instructions in the manual. There's only an information that some instruction type encodings
//  are "equivalent to the registers in the AArch32 (coproc == XX) encoding space" (XX=15 for op0=(1 or 3) and
//  XX=14 for op0=2). Perhaps let's use 16 to distinguish it from CP15 used for AArch32 and ARMv7 encodings.
#define CP_REG_ARM64_SYSREG_CP 16

//  From C5.1.2
#define CP_REG_ARM64_SYSREG_OP2_SHIFT 0
#define CP_REG_ARM64_SYSREG_CRM_SHIFT (CP_REG_ARM64_SYSREG_OP2_SHIFT + 3)
#define CP_REG_ARM64_SYSREG_CRN_SHIFT (CP_REG_ARM64_SYSREG_CRM_SHIFT + 4)
#define CP_REG_ARM64_SYSREG_OP1_SHIFT (CP_REG_ARM64_SYSREG_CRN_SHIFT + 4)
#define CP_REG_ARM64_SYSREG_OP0_SHIFT (CP_REG_ARM64_SYSREG_OP1_SHIFT + 3)
//  op0 is a 2-bit field
#define CP_REG_ARM_COPROC_SHIFT (CP_REG_ARM64_SYSREG_OP0_SHIFT + 2)

void system_instructions_and_registers_reset(CPUState *env);
void system_instructions_and_registers_init(CPUState *env, uint32_t cpu_model_id);
