/*
 * Copyright (c) 2011, Max Filippov, Open Source and Linux Lab.
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
#include "host-utils.h"
#include "osdep.h"
#include "softmmu_exec.h"
#include "tb-helper.h"

void HELPER(update_ccount)(CPUState *env)
{
    uint64_t now = tlib_get_cpu_time();

    env->ccount_time = now;
    env->sregs[CCOUNT] = env->ccount_base + (uint32_t)((now - env->time_base) * env->config->clock_freq_khz / 1000);
}

void HELPER(wsr_ccount)(CPUState *env, uint32_t v)
{
    int i;

    HELPER(update_ccount)(env);
    env->ccount_base += v - env->sregs[CCOUNT];
    for(i = 0; i < env->config->nccompare; ++i) {
        HELPER(update_ccompare)(env, i);
    }
}

void HELPER(update_ccompare)(CPUState *env, uint32_t i)
{
    uint64_t dcc;

    env->sregs[INTSET] &= ~(1u << env->config->timerint[i]);
    HELPER(update_ccount)(env);
    dcc = (uint64_t)(env->sregs[CCOMPARE + i] - env->sregs[CCOUNT] - 1) + 1;
    tlib_timer_mod(i, /*env->ccompare[i].timer,*/
                   env->ccount_time + (dcc * 1000000) / env->config->clock_freq_khz);
    env->yield_needed = 1;
}

/*!
 * Check vaddr accessibility/cache attributes and raise an exception if
 * specified by the ATOMCTL SR.
 *
 * Note: local memory exclusion is not implemented
 */
void HELPER(check_atomctl)(CPUState *env, uint32_t pc, uint32_t vaddr)
{
    uint32_t paddr, page_size;
    int access;
    uint32_t atomctl = env->sregs[ATOMCTL];
    int rc = get_physical_address(env, true, vaddr, 1, xtensa_get_cring(env), &paddr, &page_size, &access);

    /*
     * s32c1i never causes LOAD_PROHIBITED_CAUSE exceptions,
     * see opcode description in the ISA
     */
    if(rc == 0 && (access & (PAGE_READ | PAGE_WRITE)) != (PAGE_READ | PAGE_WRITE)) {
        rc = STORE_PROHIBITED_CAUSE;
    }

    if(rc) {
        HELPER(exception_cause_vaddr)(env, pc, rc, vaddr);
    }

    /*
     * When data cache is not configured use ATOMCTL bypass field.
     * See ISA, 4.3.12.4 The Atomic Operation Control Register (ATOMCTL)
     * under the Conditional Store Option.
     */
    if(!xtensa_option_enabled(env->config, XTENSA_OPTION_DCACHE)) {
        access = PAGE_CACHE_BYPASS;
    }

    switch(access & PAGE_CACHE_MASK) {
        case PAGE_CACHE_WB:
            atomctl >>= 2;
            /* fall through */
        case PAGE_CACHE_WT:
            atomctl >>= 2;
            /* fall through */
        case PAGE_CACHE_BYPASS:
            if((atomctl & 0x3) == 0) {
                HELPER(exception_cause_vaddr)(env, pc, LOAD_STORE_ERROR_CAUSE, vaddr);
            }
            break;

        case PAGE_CACHE_ISOLATE:
            HELPER(exception_cause_vaddr)(env, pc, LOAD_STORE_ERROR_CAUSE, vaddr);
            break;

        default:
            break;
    }
}

void HELPER(check_exclusive)(CPUState *env, uint32_t pc, uint32_t vaddr, uint32_t is_write)
{
    uint32_t paddr, page_size;
    int access;
    uint32_t atomctl = env->sregs[ATOMCTL];
    int rc = get_physical_address(env, true, vaddr, is_write, xtensa_get_cring(env), &paddr, &page_size, &access);

    if(rc) {
        HELPER(exception_cause_vaddr)(env, pc, rc, vaddr);
    }

    /* When data cache is not configured use ATOMCTL bypass field. */
    if(!xtensa_option_enabled(env->config, XTENSA_OPTION_DCACHE)) {
        access = PAGE_CACHE_BYPASS;
    }

    switch(access & PAGE_CACHE_MASK) {
        case PAGE_CACHE_WB:
            atomctl >>= 2;
            /* fall through */
        case PAGE_CACHE_WT:
            atomctl >>= 2;
            /* fall through */
        case PAGE_CACHE_BYPASS:
            if((atomctl & 0x3) == 0) {
                HELPER(exception_cause_vaddr)(env, pc, EXCLUSIVE_ERROR_CAUSE, vaddr);
            }
            break;

        case PAGE_CACHE_ISOLATE:
            HELPER(exception_cause_vaddr)(env, pc, LOAD_STORE_ERROR_CAUSE, vaddr);
            break;

        default:
            break;
    }
}

void HELPER(wsr_memctl)(CPUState *env, uint32_t v)
{
    if(xtensa_option_enabled(env->config, XTENSA_OPTION_ICACHE)) {
        if(extract32(v, MEMCTL_IUSEWAYS_SHIFT, MEMCTL_IUSEWAYS_LEN) > env->config->icache_ways) {
            deposit32(v, MEMCTL_IUSEWAYS_SHIFT, MEMCTL_IUSEWAYS_LEN, env->config->icache_ways);
        }
    }
    if(xtensa_option_enabled(env->config, XTENSA_OPTION_DCACHE)) {
        if(extract32(v, MEMCTL_DUSEWAYS_SHIFT, MEMCTL_DUSEWAYS_LEN) > env->config->dcache_ways) {
            deposit32(v, MEMCTL_DUSEWAYS_SHIFT, MEMCTL_DUSEWAYS_LEN, env->config->dcache_ways);
        }
        if(extract32(v, MEMCTL_DALLOCWAYS_SHIFT, MEMCTL_DALLOCWAYS_LEN) > env->config->dcache_ways) {
            deposit32(v, MEMCTL_DALLOCWAYS_SHIFT, MEMCTL_DALLOCWAYS_LEN, env->config->dcache_ways);
        }
    }
    env->sregs[MEMCTL] = v & env->config->memctl_mask;
}

uint32_t HELPER(rer)(CPUState *env, uint32_t addr)
{
    //  I think we should implement it at C# level?
    tlib_abortf("reading from external register not yet supported");
    return 0;
    /*
    return address_space_ldl(env->address_space_er, addr,
                             MEMTXATTRS_UNSPECIFIED, NULL);
    */
}

void HELPER(wer)(CPUState *env, uint32_t data, uint32_t addr)
{
    //  I think we should implement it at C# level?
    tlib_abortf("writing to external register not yet supported");
    /*
    address_space_stl(env->address_space_er, addr, data,
                      MEMTXATTRS_UNSPECIFIED, NULL);
    */
}

void HELPER(simcall)(CPUState *env)
{
    tlib_do_semihosting();
}
