/*
 * ARM common
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

#pragma once

#include "cpu.h"

static inline TCGv load_cpu_offset(int offset)
{
    TCGv tmp = tcg_temp_local_new_i32();
    tcg_gen_ld_i32(tmp, cpu_env, offset);
    return tmp;
}

#define load_cpu_field(name) load_cpu_offset(offsetof(CPUState, name))

static inline void store_cpu_offset(TCGv var, int offset)
{
    tcg_gen_st_i32(var, cpu_env, offset);
    tcg_temp_free_i32(var);
}

#define store_cpu_field(var, name) store_cpu_offset(var, offsetof(CPUState, name))

static inline TCGv gen_ld32(TCGv addr, int index)
{
    TCGv tmp = tcg_temp_local_new_i32();
    tcg_gen_qemu_ld32u(tmp, addr, index);
    return tmp;
}

static inline long vfp_reg_offset(enum arm_fp_precision precision, int reg)
{
    if(precision == DOUBLE_PRECISION) {
        return offsetof(CPUState, vfp.regs[reg]);
    } else if(reg & 1) {
        return offsetof(CPUState, vfp.regs[reg >> 1]) + offsetof(CPU_DoubleU, l.upper);
    } else {
        return offsetof(CPUState, vfp.regs[reg >> 1]) + offsetof(CPU_DoubleU, l.lower);
    }
}

/* Return the offset of a Qn register */
static inline long mve_qreg_offset(uint32_t reg)
{
    return vfp_reg_offset(DOUBLE_PRECISION, reg * 2);
}

/* Return the offset of a specified lane of Qn register
   It correspond with H macros so:
   size 1 - 8 bits per lane
   size 2 - 16 bits per lane
   size 4 - 32 bits per lane */
#define mve_qreg_lane_offset(size, reg, lane_number) (mve_qreg_offset(reg) + (H##size(lane_number) * size))

#define mve_qreg_lane_offset_b(reg, lane_number) mve_qreg_lane_offset(1, reg, lane_number)
#define mve_qreg_lane_offset_h(reg, lane_number) mve_qreg_lane_offset(2, reg, lane_number)
#define mve_qreg_lane_offset_w(reg, lane_number) mve_qreg_lane_offset(4, reg, lane_number)

static inline uint8_t clz_u8(uint8_t a)
{
    uint8_t count = 0;
    while((a & 0x80) == 0 && count < 8) {
        a <<= 1;
        count++;
    }
    return count;
}

static inline uint16_t clz_u16(uint16_t a)
{
    uint16_t count = 0;
    while((a & 0x8000) == 0 && count < 16) {
        a <<= 1;
        count++;
    }
    return count;
}

static inline uint8_t cls_s8(uint8_t a)
{
    uint8_t count = 0;
    const uint8_t sign = !!(a & 0x80);
    while(!!(a & 0x40) == sign && count < 7) {
        a <<= 1;
        count++;
    }
    return count;
}

static inline uint16_t cls_s16(uint16_t a)
{
    uint16_t count = 0;
    const uint16_t sign = !!(a & 0x8000);
    while(!!(a & 0x4000) == sign && count < 15) {
        a <<= 1;
        count++;
    }
    return count;
}
