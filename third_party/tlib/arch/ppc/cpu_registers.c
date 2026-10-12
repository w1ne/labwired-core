/*
 *  PPC registers interface.
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

#ifdef TARGET_PPC64
uint64_t *get_reg_pointer_64(int reg)
{
    switch(reg) {
        case R_0_64 ... R_31_64:
            return &(cpu->gpr[reg - R_0_64]);
        case NIP_64:
            return &(cpu->nip);
        case MSR_64:
            return &(cpu->msr);
        case LR_64:
            return &(cpu->lr);
        case CTR_64:
            return &(cpu->ctr);
        case XER_64:
            return &(cpu->xer);
        default:
            return NULL;
    }
}
CPU_REGISTER_ACCESSOR(64);
#endif
#ifdef TARGET_PPC32
uint32_t *get_reg_pointer_32(int reg)
{
    switch(reg) {
        case R_0_32 ... R_31_32:
            return &(cpu->gpr[reg - R_0_32]);
        case NIP_32:
            return &(cpu->nip);
        case MSR_32:
            return &(cpu->msr);
        case LR_32:
            return &(cpu->lr);
        case CTR_32:
            return &(cpu->ctr);
        case XER_32:
            return &(cpu->xer);
        default:
            return NULL;
    }
}
CPU_REGISTER_ACCESSOR(32);
#endif
