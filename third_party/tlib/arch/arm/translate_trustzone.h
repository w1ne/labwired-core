/*
 *  ARM translation for ARM’s TrustZone extension (TZ)
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

static inline bool is_insn_sg_half(uint32_t insn)
{
    return (insn & 0xffff) == 0xe97f;
}

static inline bool is_insn_sg(uint32_t insn)
{
    //  Both halves of SG instruction are the same
    return is_insn_sg_half(insn >> 16) && is_insn_sg_half(insn);
}
