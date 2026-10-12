/*
 *  ARM translation for ARM’s floating-point extension (VFP)
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

#pragma once

static inline bool is_insn_vminmaxnm(uint32_t insn)
{
    return (insn & 0x0fb00c10) == 0x0e800800;
}

static inline bool is_insn_vsel(uint32_t insn)
{
    return (insn & 0xff800c50) == 0xfe000800;
}

static inline bool is_insn_vcvt(uint32_t insn)
{
    return (insn & 0x0fbc0c50) == 0x0ebc0840;
}

static inline bool is_insn_vrint(uint32_t insn)
{
    return (insn & 0x0fbc0cd0) == 0x0eb80840;
}

static inline bool is_insn_vinsmovx(uint32_t insn)
{
    return (insn & 0x0fbf0f50) == 0x0eb00a40;
}
