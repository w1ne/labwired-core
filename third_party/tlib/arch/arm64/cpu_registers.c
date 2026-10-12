/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#include <stdint.h>

#include "cpu.h"
#include "cpu_registers.h"
#include "unwind.h"

uint64_t tlib_get_register_value_64(int reg_number)
{
    //  TODO: AArch64 to AArch32 mappings (R8_fiq == W24, SP_irq == W17 etc.):
    //        https://developer.arm.com/documentation/den0024/a/ARMv8-Registers/Changing-execution-state--again-/Registers-at-AArch32
    switch(reg_number) {
        case CPSR_32:
            return cpsr_read(cpu);
        case PSTATE_32:
            return pstate_read(cpu);
        case FPCR_32:
            return vfp_get_fpcr(cpu);
        case FPSR_32:
            return vfp_get_fpsr(cpu);
        case R_0_32 ... R_15_32:
            return cpu->regs[reg_number - R_0_32];
        case D_0_64 ... D_31_64:
            return cpu->vfp.zregs[reg_number - D_0_64].d[0];
        case PC_64:
            //  The PC register's index is the same for both AArch32 and AArch64.
            return is_a64(cpu) ? cpu->pc : cpu->regs[15];
        case SP_64:
            //  The SP register's index is the same for both AArch32 and AArch64.
            return is_a64(cpu) ? cpu->xregs[31] : cpu->regs[13];
        case X_0_64 ... X_30_64:
            return cpu->xregs[reg_number - X_0_64];
    }

    tlib_abortf("Read from undefined CPU register number %d detected", reg_number);
    __builtin_unreachable();
}

EXC_INT_1(uint64_t, tlib_get_register_value_64, int, reg_number)

void tlib_set_register_value_64(int reg_number, uint64_t value)
{
    switch(reg_number) {
        case CPSR_32:
            cpsr_write(cpu, (uint32_t)value, 0xFFFFFFFF, CPSRWriteRaw);
            arm_rebuild_hflags(cpu);
            return;
        case PSTATE_32:
            pstate_write(cpu, (uint32_t)value);
            arm_rebuild_hflags(cpu);
            return;
        case FPCR_32:
            vfp_set_fpcr(cpu, (uint32_t)value);
            return;
        case FPSR_32:
            vfp_set_fpsr(cpu, (uint32_t)value);
            return;
        case R_0_32 ... R_15_32:
            cpu->regs[reg_number - R_0_32] = (uint32_t)value;
            return;
        case PC_64:
            //  The PC register's index is the same for both AArch32 and AArch64.
            if(is_a64(cpu)) {
                cpu->pc = value;
            } else {
                cpu->regs[15] = (uint32_t)value;
            }
            return;
        case SP_64:
            //  The SP register's index is the same for both AArch32 and AArch64.
            if(is_a64(cpu)) {
                cpu->xregs[31] = value;
            } else {
                cpu->regs[13] = (uint32_t)value;
            }
            return;
        case X_0_64 ... X_30_64:
            cpu->xregs[reg_number - X_0_64] = value;
            return;
    }

    tlib_abortf("Write to undefined CPU register number %d detected", reg_number);
}

EXC_VOID_2(tlib_set_register_value_64, int, reg_number, uint64_t, value)

uint32_t tlib_get_register_value_32(int reg_number)
{
    return (uint32_t)tlib_get_register_value_64(reg_number);
}

EXC_INT_1(uint32_t, tlib_get_register_value_32, int, reg_number)

void tlib_set_register_value_32(int reg_number, uint32_t value)
{
    tlib_set_register_value_64(reg_number, value);
}

EXC_VOID_2(tlib_set_register_value_32, int, reg_number, uint32_t, value)
