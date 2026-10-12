/*
 *  X86 registers interface.
 *
 *  Copyright (c) Antmicro
 *  Copyright (c) Realtime Embedded
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

#ifdef TARGET_X86_64
uint64_t *get_reg_pointer_64(int reg)
{
    switch(reg) {
        case RIP_64:
            return &(cpu->eip);
        case EFLAGS_64:
            return &(cpu->eflags);
        case CS_64:
            return &(cpu->segs[R_CS].base);
        case SS_64:
            return &(cpu->segs[R_SS].base);
        case DS_64:
            return &(cpu->segs[R_DS].base);
        case ES_64:
            return &(cpu->segs[R_ES].base);
        case FS_64:
            return &(cpu->segs[R_FS].base);
        case GS_64:
            return &(cpu->segs[R_GS].base);
        case RAX_64 ... R15_64:
            return &(cpu->regs[reg]);
        case CR0_64 ... CR4_64:
            return &(cpu->cr[reg - CR0_64]);
        case ST0_64 ... ST7_64:
            return &ST(reg - ST7_64).low;
        default:
            return NULL;
    }
}

CPU_REGISTER_ACCESSOR(64)
#else
uint32_t *get_reg_pointer_32(int reg)
{
    switch(reg) {
        case EIP_32:
            return &(cpu->eip);
        case EFLAGS_32:
            return &(cpu->eflags);
        case CS_32:
            return &(cpu->segs[R_CS].base);
        case SS_32:
            return &(cpu->segs[R_SS].base);
        case DS_32:
            return &(cpu->segs[R_DS].base);
        case ES_32:
            return &(cpu->segs[R_ES].base);
        case FS_32:
            return &(cpu->segs[R_FS].base);
        case GS_32:
            return &(cpu->segs[R_GS].base);
        default:
            if(reg >= EAX_32 && reg <= EDI_32) {
                return &(cpu->regs[reg]);
            }
            if(reg >= CR0_32 && reg <= CR4_32) {
                return &(cpu->cr[reg - CR0_32]);
            }
            return NULL;
    }
}

CPU_REGISTER_ACCESSOR(32)
#endif
