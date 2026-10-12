/*
 *  SPARC registers interface.
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

uint32_t *get_reg_pointer_32(int reg)
{
    switch(reg) {
        case R_0_32 ... R_7_32: /* R0-R7 aka Global0-7 */
            return &(cpu->gregs[reg]);
        case R_8_32 ... R_31_32: /* R8 to 31 : Out0-7, Local0-7, In0-7 */
            return &(cpu->regwptr[reg - R_8_32]);
        case PSR_32:
            /* Compute PSR before exposing state.  */
            if(cpu->cc_op != CC_OP_FLAGS) {
                cpu_get_psr(cpu);
            }
            return &(cpu->psr);
        case TBR_32:
            return &(cpu->tbr);
        case Y_32:
            return &(cpu->y);
        case PC_32:
            return &(cpu->pc);
        case NPC_32:
            return &(cpu->npc);
        case ASR_16_32 ... ASR_31_32:
            return &(cpu->asr[reg - ASR_16_32]);
        case WIM_32:
            return &(cpu->wim);
        case FSR_32:
            return &(cpu->fsr);
        case CSR_32:
            return &(cpu->csr);
        default:
            return NULL;
    }
}

CPU_REGISTER_ACCESSOR(32)
