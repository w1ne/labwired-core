/*
 *  SPARC interface functions.
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
#include "unwind.h"

/* Add CPU slot number info to ASR17 register, */
/* bit field [31:28] is for processor index */
void tlib_set_slot(uint32_t slot)
{
    unsigned int asr17;
    /* Default value is set for core 0, */
    /* only update ASR17 for slave cores 1-15 */
    if((slot > 0) && slot < 16) {
        asr17 = (unsigned int)((cpu->asr[1] & 0xFFFFFFF) + ((slot << 28) & 0xF0000000));
        cpu->asr[1] = asr17;
    }
}

EXC_VOID_1(tlib_set_slot, uint32_t, slot)

void tlib_set_entry_point(uint32_t entry_point)
{
    cpu->pc = entry_point;
    cpu->npc = cpu->pc + 4;
}

EXC_VOID_1(tlib_set_entry_point, uint32_t, entry_point)

void tlib_clear_wfi()
{
    cpu->wfi = 0;
}

EXC_VOID_0(tlib_clear_wfi)

void tlib_set_wfi()
{
    cpu->wfi = 1;
}

EXC_VOID_0(tlib_set_wfi)
