/*
 * Copyright (c) 2011 - 2019, Max Filippov, Open Source and Linux Lab.
 * All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 *     * Redistributions of source code must retain the above copyright
 *       notice, this list of conditions and the following disclaimer.
 *     * Redistributions in binary form must reproduce the above copyright
 *       notice, this list of conditions and the following disclaimer in the
 *       documentation and/or other materials provided with the distribution.
 *     * Neither the name of the Open Source and Linux Lab nor the
 *       names of its contributors may be used to endorse or promote products
 *       derived from this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
 * AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 * IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
 * ARE DISCLAIMED. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY
 * DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
 * (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
 * LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
 * ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 * (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
 * SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */

#include "cpu.h"
#include "osdep.h"
#include "tb-helper.h"

void HELPER(wsr_ibreakenable)(CPUState *env, uint32_t v)
{
    tlib_printf(LOG_LEVEL_WARNING, "Setting watchpoints with IBREAK instructions isn't supported!");
    env->sregs[IBREAKENABLE] = v & ((1 << env->config->nibreak) - 1);
}

void HELPER(wsr_ibreaka)(CPUState *env, uint32_t i, uint32_t v)
{
    tlib_printf(LOG_LEVEL_WARNING, "Setting watchpoints with IBREAK instructions isn't supported!");
    env->sregs[IBREAKA + i] = v;
}

void HELPER(wsr_dbreaka)(CPUState *env, uint32_t i, uint32_t v)
{
    tlib_printf(LOG_LEVEL_WARNING, "Setting watchpoints with DBREAK instructions isn't supported!");
    env->sregs[DBREAKA + i] = v;
}

void HELPER(wsr_dbreakc)(CPUState *env, uint32_t i, uint32_t v)
{
    tlib_printf(LOG_LEVEL_WARNING, "Setting watchpoints with DBREAK instructions isn't supported!");
    env->sregs[DBREAKC + i] = v;
}
