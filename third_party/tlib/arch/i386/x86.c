/*
 *  X86-specific interface functions.
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
#include "infrastructure.h"
#include "arch_callbacks.h"

#ifdef TARGET_I386

#define APIC_BSP       (1 << 8)
#define x2APIC_ENABLED (1 << 10)
#define xAPIC_ENABLED  (1 << 11)
#define APIC_BASE      (0xffffff << 12)

int cpu_is_bsp(CPUState *env)
{
    return env->apic_state & APIC_BSP;
}

uint64_t cpu_get_apic_base(CPUState *env)
{
    return env->apic_state;
}

void apic_init_reset(CPUState *env)
{
    env->apic_state = (0xFEE00 << 12) | xAPIC_ENABLED;
}

void cpu_smm_update(CPUState *env)
{
    tlib_printf(LOG_LEVEL_WARNING, "%s(...)", __FUNCTION__);
}

void cpu_set_ferr(CPUState *env)
{
    tlib_printf(LOG_LEVEL_WARNING, "%s(...)", __FUNCTION__);
}

//  task priority register
void cpu_set_apic_tpr(CPUState *env, uint8_t val)
{
    tlib_printf(LOG_LEVEL_WARNING, "%s(%X)", __FUNCTION__, val);
}

void cpu_set_apic_base(CPUState *env, uint64_t val)
{
    env->apic_state = val;
    tlib_set_apic_base_value(val);
}

int cpu_get_pic_interrupt(CPUState *env)
{
    return tlib_get_pending_interrupt();
}

//  task priority register
uint8_t cpu_get_apic_tpr(CPUState *env)
{
    tlib_printf(LOG_LEVEL_WARNING, "%s(...)", __FUNCTION__);
    return 0;
}

void apic_sipi(CPUState *env)
{
    tlib_printf(LOG_LEVEL_WARNING, "%s(...)", __FUNCTION__);
}

uint64_t cpu_get_tsc(CPUState *env)
{
    return tlib_get_total_elapsed_cycles() + env->tsc_offset;
}

#endif
