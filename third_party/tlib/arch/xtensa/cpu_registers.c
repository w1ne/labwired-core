/*
 *  Xtensa registers interface
 *
 *  Copyright (c) Antmicro
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see <http://www.gnu.org/licenses/>.
 */
#include <stdint.h>

#include "cpu.h"
#include "cpu_registers.h"
#include "unwind.h"

//  REMARK: here we use #ifdef/#endif,#ifdef/#endif notation just to be consistent with header file; in header it is required by
//  our parser

uint32_t *get_reg_pointer_32(int reg)
{
    switch(reg) {
        case A_0_32 ... A_15_32:
            return &(cpu->regs[reg - A_0_32]);
        case AR_0_32 ... AR_31_32:
            return &(cpu->phys_regs[reg - AR_0_32]);
        case PC_32:
            return &(cpu->pc);
        case SAR_32:
            return &(cpu->sregs[SAR]);

        //  User Registers.
        case EXPSTATE_32:
            return &(cpu->uregs[EXPSTATE]);

        //  Special Registers. The sregs index can also be found in GDB's
        //  'xtensa-config.c' as a less significant byte of XTREG's 'targno'.
        case ATOMCTL_32:
            return &(cpu->sregs[ATOMCTL]);
        case CCOMPARE_0_32 ... CCOMPARE_2_32:
            return &(cpu->sregs[CCOMPARE + reg - CCOMPARE_0_32]);
        case CCOUNT_32:
            return &(cpu->sregs[CCOUNT]);
        case CONFIGID_0_32:
            return &(cpu->sregs[CONFIGID0]);
        case CONFIGID_1_32:
            return &(cpu->sregs[CONFIGID1]);
        case DBREAKA_0_32 ... DBREAKA_1_32:
            return &(cpu->sregs[DBREAKA + reg - DBREAKA_0_32]);
        case DBREAKC_0_32 ... DBREAKC_1_32:
            return &(cpu->sregs[DBREAKC + reg - DBREAKC_0_32]);
        case DDR_32:
            return &(cpu->sregs[DDR]);
        case DEBUGCAUSE_32:
            return &(cpu->sregs[DEBUGCAUSE]);
        case DEPC_32:
            return &(cpu->sregs[DEPC]);
        case EPC_1_32 ... EPC_7_32:
            return &(cpu->sregs[EPC1 + reg - EPC_1_32]);
        case EPS_2_32 ... EPS_7_32:
            return &(cpu->sregs[EPS2 + reg - EPS_2_32]);
        case EXCCAUSE_32:
            return &(cpu->sregs[EXCCAUSE]);
        case EXCSAVE_1_32 ... EXCSAVE_7_32:
            return &(cpu->sregs[EXCSAVE1 + reg - EXCSAVE_1_32]);
        case EXCVADDR_32:
            return &(cpu->sregs[EXCVADDR]);
        case IBREAKA_0_32 ... IBREAKA_1_32:
            return &(cpu->sregs[IBREAKA + reg - IBREAKA_0_32]);
        case IBREAKENABLE_32:
            return &(cpu->sregs[IBREAKENABLE]);
        case ICOUNT_32:
            return &(cpu->sregs[ICOUNT]);
        case ICOUNTLEVEL_32:
            return &(cpu->sregs[ICOUNTLEVEL]);
        case INTCLEAR_32:
            return &(cpu->sregs[INTCLEAR]);
        case INTENABLE_32:
            return &(cpu->sregs[INTENABLE]);
        //  Read: INTERRUPT. Write: INTSET.
        case INTERRUPT_32 ... INTSET_32:
            return &(cpu->sregs[INTSET]);
        case MISC_0_32 ... MISC_1_32:
            return &(cpu->sregs[MISC + reg - MISC_0_32]);
        case MMID_32:
            return &(cpu->sregs[MMID]);
        case PRID_32:
            return &(cpu->sregs[PRID]);
        case PS_32:
            return &(cpu->sregs[PS]);
        case SCOMPARE_1_32:
            return &(cpu->sregs[SCOMPARE1]);
        case VECBASE_32:
            return &(cpu->sregs[VECBASE]);
        case WINDOWBASE_32:
            return &(cpu->sregs[WINDOW_BASE]);
        case WINDOWSTART_32:
            return &(cpu->sregs[WINDOW_START]);
        default:
            break;
    }
    return NULL;
}

uint32_t get_masked_register_value_32(int reg_number, int offset, int width)
{
    uint32_t *ptr = get_reg_pointer_32(reg_number);
    return extract32(*ptr, offset, width);
}

uint32_t tlib_get_register_value_32(int reg_number)
{
    //  Sync A --> AR to have the latest windowed register values.
    if(reg_number >= AR_0_32 && reg_number <= AR_31_32) {
        xtensa_sync_phys_from_window(cpu);
    }

    switch(reg_number) {
        case PSCALLINC_32:
            return get_masked_register_value_32(PS_32, 16, 2);
        case PSEXCM_32:
            return get_masked_register_value_32(PS_32, 4, 1);
        case PSINTLEVEL_32:
            return get_masked_register_value_32(PS_32, 0, 4);
        case PSOWB_32:
            return get_masked_register_value_32(PS_32, 8, 4);
        case PSUM_32:
            return get_masked_register_value_32(PS_32, 5, 1);
        case PSWOE_32:
            return get_masked_register_value_32(PS_32, 18, 1);
    }

    uint32_t *ptr = get_reg_pointer_32(reg_number);
    if(ptr == NULL) {
        tlib_abortf("Read from undefined CPU register number %d detected", reg_number);
    }

    return *ptr;
}

EXC_INT_1(uint32_t, tlib_get_register_value_32, int, reg_number)

void set_masked_register_value_32(int reg_number, int offset, int width, uint32_t value)
{
    uint32_t *ptr = get_reg_pointer_32(reg_number);
    *ptr = deposit32(*ptr, offset, width, value);
}

void tlib_set_register_value_32(int reg_number, uint32_t value)
{
    switch(reg_number) {
        case PSCALLINC_32:
            set_masked_register_value_32(PS_32, 16, 2, value);
            return;
        case PSEXCM_32:
            set_masked_register_value_32(PS_32, 4, 1, value);
            return;
        case PSINTLEVEL_32:
            set_masked_register_value_32(PS_32, 0, 4, value);
            return;
        case PSOWB_32:
            set_masked_register_value_32(PS_32, 8, 4, value);
            return;
        case PSUM_32:
            set_masked_register_value_32(PS_32, 5, 1, value);
            return;
        case PSWOE_32:
            set_masked_register_value_32(PS_32, 18, 1, value);
            return;
    }

    uint32_t *ptr = get_reg_pointer_32(reg_number);
    if(ptr == NULL) {
        tlib_abortf("Write to undefined CPU register number %d detected", reg_number);
    }

    *ptr = value;
}

EXC_VOID_2(tlib_set_register_value_32, int, reg_number, uint32_t, value)
