/*
 *  PPC interface functions.
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

int32_t tlib_set_pending_interrupt(int32_t interruptNo, int32_t level)
{
    if(level) {
        cpu->pending_interrupts |= 1 << interruptNo;
    } else {
        cpu->pending_interrupts &= ~(1 << interruptNo);
        if(cpu->pending_interrupts == 0) {
            return 1;
        }
    }
    return 0;
}

EXC_INT_2(int32_t, tlib_set_pending_interrupt, int32_t, interruptNo, int32_t, level)

void tlib_set_little_endian_mode(bool mode)
{
    if(mode) {
        cpu->hflags |= 1 << MSR_LE;
        cpu->msr |= 1 << MSR_LE;
    } else {
        cpu->hflags &= ~(1 << MSR_LE);
        cpu->msr &= ~(1 << MSR_LE);
    }
}

EXC_VOID_1(tlib_set_little_endian_mode, bool, mode)
