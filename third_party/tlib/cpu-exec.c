/*
 *  i386 emulator main execution loop
 *
 *  Copyright (c) 2003-2005 Fabrice Bellard
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
#include "hash-table-store-test.h"
#include <stdatomic.h>

#include "cpu.h"
#include "infrastructure.h"
#include "tcg.h"
#include "atomic.h"
#include "tlib-alloc.h"

static bool virt_to_phys_mmu(target_ulong virtual, uint32_t access_type, bool nofault, uint16_t mmu_idx, target_ulong *physical)
{
    target_ulong masked_virtual = virtual & TARGET_PAGE_MASK;
    target_ulong page_index = (virtual >> TARGET_PAGE_BITS) & (CPU_TLB_SIZE - 1);

    CPUTLBEntry *entry = &env->tlb_table[mmu_idx][page_index];
    target_ulong *addr = &entry->addrs[access_type];

    if(*addr != -1 && (*addr & TLB_ONE_SHOT)) {
        //  TLB_ONE_SHOT pages should not be reused
        //  as there might be protected memory regions in them.
        //  A protected memory region does not have to fill the whole page;
        //  there might also be many memory regions defined for a single page.
        //  That's why we flush the page and force
        //  calling tlb_fill to check memory region
        //  restrictions on each access.
        //  This code can only be called when the CPU is not running, so we can say we're from generated code
        tlb_flush_page(env, *addr, true);
    }

    if((*addr & TARGET_PAGE_MASK) != masked_virtual) {
        //  Refresh page table from hardware tables
        if(tlb_fill(env, virtual, access_type, mmu_idx, NULL /* retaddr */, nofault, 1) != TRANSLATE_SUCCESS) {
            return false;
        }
        if(unlikely((*addr & TARGET_PAGE_MASK) != masked_virtual)) {
            return false;
        }
    }

    /* The iotlb entry contains the physical/RAM page offset both for MMIO and normal RAM.
     * We use it for normal RAM too because Renode host memory blocks are discovered lazily,
     * making reverse host-pointer lookups unreliable. */
    *physical = virtual + (env->iotlb[mmu_idx][page_index] & TARGET_PAGE_MASK);
    return true;
}

/*
 * Translates a virtual address to a physical one, according to CPU memory mappings.
 * The CPU must not be running when this function is called.
 */
target_ulong virt_to_phys(target_ulong virtual, uint32_t access_type, uint32_t nofault)
{
    target_ulong physical;
    uint16_t mmu_idx = cpu_mmu_index(env);

    nofault = !!nofault;
    /* Access types :
     *      0 : read
     *      1 : write
     *      2 : instr fetch */
    if(access_type > 2) {
        return -1;
    }
    if(virt_to_phys_mmu(virtual, access_type, nofault, mmu_idx, &physical)) {
        return physical;
    }
    //  Not mapped in current env mmu mode, check other modes
    for(uint16_t idx = 0; idx < NB_MMU_MODES; idx++) {
        if(idx == mmu_idx) {
            //  Already checked
            continue;
        }
        if(virt_to_phys_mmu(virtual, access_type, nofault, idx, &physical)) {
            tlib_printf(LOG_LEVEL_WARNING,
                        "Using incorrect MMU mode for translation of address 0x" TARGET_FMT_lx
                        ", current MMU mode is %d, mapping is for MMU mode %d",
                        virtual, mmu_idx, idx);
            return physical;
        }
    }
    return -1;
}

int tb_invalidated_flag;

void TLIB_NORETURN cpu_loop_exit_without_hook(CPUState *env)
{
    env->current_tb = NULL;
    longjmp(env->jmp_env, 1);
}

void TLIB_NORETURN cpu_loop_exit(CPUState *env)
{
    if(env->block_finished_hook_present) {
        TranslationBlock *tb = env->current_tb;
        target_ulong pc = CPU_PC(env);
        target_ulong data[TARGET_INSN_START_WORDS];
        int executed_instructions =
            cpu_get_data_for_pc(env, tb, pc, /* pc_is_host */ false, data, /* skip_current_instruction */ false);
        /* PC points to the next instruction to execute, so if the exit happened on the last instruction
         * it will point outside of the TB, so substitute the full icount in that case; otherwise decrement
         * the count by one */
        tlib_on_block_finished(tb->pc, executed_instructions == -1 ? tb->icount : executed_instructions - 1);
    }
    cpu_loop_exit_without_hook(env);
}

void TLIB_NORETURN cpu_loop_exit_restore(CPUState *cpu, uintptr_t pc, uint32_t call_hook)
{
    TranslationBlock *tb = NULL;
    uint32_t executed_instructions = 0;
    if(pc) {
        tb = tb_find_pc(pc);
        if(!tb) {
            tlib_abortf("tb_find_pc for pc = 0x%lx failed!", pc);
        }
        executed_instructions = cpu_restore_state_and_restore_instructions_count(cpu, tb, pc, true);
    }
    if(call_hook && cpu->block_finished_hook_present) {
        tlib_on_block_finished(tb ? tb->pc : CPU_PC(cpu), executed_instructions);
    }

    cpu_loop_exit_without_hook(cpu);
}

static TranslationBlock *tb_find_slow(CPUState *env, target_ulong pc, target_ulong cs_base, uint64_t flags)
{
    tlib_on_translation_block_find_slow(pc);
    TranslationBlock *tb, **ptb1;
    TranslationBlock *prev_related_tb;
    unsigned int h;
    tb_page_addr_t phys_page1;
    target_ulong virt_page2;
    uint32_t max_icount;

    tb_invalidated_flag = 0;
    prev_related_tb = NULL;
    max_icount = env->instructions_count_limit - env->instructions_count_value;

    /* find translated block using physical mappings */
    phys_page1 = get_page_addr_code(env, pc, true);
    /* tb_phys_hash_func expects the physical PC to be passed.
     * `phys_page1` will not be the physical PC if it resides in an `ArrayMemory` peripheral
     * (see logic in `get_page_addr_code`) */
    h = tb_phys_hash_func(phys_page1 | (pc & ~TARGET_PAGE_MASK));
    ptb1 = &tb_phys_hash[h];

    if(unlikely(env->tb_cache_disabled)) {
        goto not_found;
    }

    for(;;) {
        tb = *ptb1;
        if(!tb) {
            goto not_found;
        }
        if(tb->pc == pc && tb->page_addr[0] == phys_page1 && tb->cs_base == cs_base && tb->flags == flags) {
            if(tb->icount <= max_icount) {
                /* check next page if needed */
                if(tb->page_addr[1] != -1) {
                    tb_page_addr_t phys_page2;

                    virt_page2 = (pc & TARGET_PAGE_MASK) + TARGET_PAGE_SIZE;
                    phys_page2 = get_page_addr_code(env, virt_page2, true);
                    if(tb->page_addr[1] == phys_page2) {
                        goto found;
                    }
                } else {
                    goto found;
                }
            } else {
                prev_related_tb = tb;
            }
        }
        ptb1 = &tb->phys_hash_next;
    }
not_found:
    /* if no translated code available, then translate it now */
    tb = tb_gen_code(env, pc, cs_base, flags, 0);

    /* if tb_gen_code flushed translation blocks, ptb1 and prev_related_tb can be invalid;
     * this is indicated by `tb_invalidated_flag` which is reset at the beginning of the current function
     * and set when we invalidate TB (e.g. as a result of the code buffer reallocation) */
    if(unlikely(tb_invalidated_flag)) {
        prev_related_tb = NULL;
        ptb1 = &tb_phys_hash[h];
    }
found:
    /* Move the last found TB closer to the beginning of the list,
     * but keeping related TBs sorted.
     * 'related' means TBs generated from the same code, but with
     * different icount */
    if(!tb->was_cut || !prev_related_tb) {
        /* TB wasn't cut or is the first in the list among related to it.
         * It means it is the largest TB and can be
         * inserted at the beginning of the list */
        *ptb1 = tb->phys_hash_next;
        tb->phys_hash_next = tb_phys_hash[h];
        tb_phys_hash[h] = tb;
    } else {
        /* Move the TB just after previous related TB */
        *ptb1 = tb->phys_hash_next;
        tb->phys_hash_next = prev_related_tb->phys_hash_next;
        prev_related_tb->phys_hash_next = tb;
    }
    /* we add the TB in the virtual pc hash table */
    env->tb_jmp_cache[tb_jmp_cache_hash_func(pc)] = tb;

    return tb;
}

static inline TranslationBlock *tb_find_fast(CPUState *env)
{
    TranslationBlock *tb;
    target_ulong cs_base, pc;
    int flags;

    uint32_t max_icount = (env->instructions_count_limit - env->instructions_count_value);

    /* we record a subset of the CPU state. It will
       always be the same before a given translated block
       is executed. */
    cpu_get_tb_cpu_state(env, &pc, &cs_base, &flags);
    tb = env->tb_jmp_cache[tb_jmp_cache_hash_func(pc)];
    if(unlikely(!tb || tb->pc != pc || tb->cs_base != cs_base || tb->flags != flags || env->tb_cache_disabled) ||
       (tb->was_cut && tb->icount < max_icount) || (tb->icount > max_icount)) {
        tb = tb_find_slow(env, pc, cs_base, flags);
    }
    return tb;
}

static CPUDebugExcpHandler *debug_excp_handler;

CPUDebugExcpHandler *cpu_set_debug_excp_handler(CPUDebugExcpHandler *handler)
{
    CPUDebugExcpHandler *old_handler = debug_excp_handler;

    debug_excp_handler = handler;
    return old_handler;
}

//  `was_not_working` is in `env` so it will reset when CPU is reset
//  and so it behaves properly after deserialization
inline static void on_cpu_no_work(CPUState *const env)
{
    if(!env->was_not_working) {
        env->was_not_working = true;
        //  Report WFI enter
        tlib_on_wfi_state_change(true);
    }
}

inline static void on_cpu_has_work(CPUState *const env)
{
    if(env->was_not_working) {
        env->was_not_working = false;
        //  Report WFI exit
        tlib_on_wfi_state_change(false);
    }
}

/* main execution loop */

int process_interrupt(int interrupt_request, CPUState *env);
void cpu_exec_prologue(CPUState *env);
void cpu_exec_epilogue(CPUState *env);

#ifndef __llvm__
//  it looks like cpu_exec is aware of possible problems and restores `env`, so the warning is not necessary
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wclobbered"
#endif
int cpu_exec(CPUState *env)
{
    int ret, interrupt_request;
    TranslationBlock *tb;
    uint8_t *tc_ptr;
    uintptr_t next_tb;

#ifdef TARGET_PROTO_ARM_M
    if(env->v7m.locked_up &&
       (!(env->interrupt_request & CPU_INTERRUPT_HARD) || tlib_nvic_find_pending_irq() != ARMV7M_EXCP_NMI)) {
        /* Armv8-M ARM rules RMBTM and RHTVD: fetch, execution, and ITSTATE
         * advancement stop while the PE is in Lockup. */
        return EXCP_LOCKUP;
    }
#endif

    if(!cpu_has_work(env)) {
        if(unlikely(env->cpu_wfi_state_change_hook_present)) {
            on_cpu_no_work(env);
        }
        return EXCP_WFI;
    }
    if(unlikely(env->cpu_wfi_state_change_hook_present)) {
        on_cpu_has_work(env);
    }

    cpu_exec_prologue(env);
    env->exception_index = -1;

    /* prepare setjmp context for exception handling */
    for(;;) {
        unlock_dangling_locks(env);
        if(setjmp(env->jmp_env) == 0) {
            /* if an exception is pending, we execute it here */
            if(env->exception_index >= 0) {
                if(env->return_on_exception || env->exception_index >= EXCP_INTERRUPT) {
                    /* exit request from the cpu execution loop */
                    ret = env->exception_index;
                    if((ret == EXCP_DEBUG) && debug_excp_handler) {
                        debug_excp_handler(env);
                    }
                    break;
                } else {
                    do_interrupt(env);
                    if(env->exception_index != -1) {
                        if(env->exception_index == EXCP_LOCKUP) {
                            ret = EXCP_LOCKUP;
                            break;
                        }
                        if(env->exception_index == EXCP_WFI) {
                            env->exception_index = -1;
                            ret = 0;
                            break;
                        }
                        env->exception_index = -1;
                    }
                }
            }

            next_tb = 0; /* force lookup of first TB */
            for(;;) {
                cpu_sync_instructions_count(env);

                interrupt_request = env->interrupt_request;
                if(unlikely(interrupt_request)) {
                    if(is_interrupt_pending(env, CPU_INTERRUPT_DEBUG)) {
                        clear_interrupt_pending(env, CPU_INTERRUPT_DEBUG);
                        env->exception_index = EXCP_DEBUG;
                        cpu_loop_exit_without_hook(env);
                    }
                    if(process_interrupt(interrupt_request, env)) {
                        next_tb = 0;
                    }
                    if(env->exception_index == EXCP_DEBUG || env->exception_index == EXCP_WFI) {
                        cpu_loop_exit_without_hook(env);
                    }
                    env->exception_index = -1;
                    /* Don't use the cached interrupt_request value,
                       do_interrupt may have updated the EXITTB flag. */
                    if(is_interrupt_pending(env, CPU_INTERRUPT_EXITTB)) {
                        clear_interrupt_pending(env, CPU_INTERRUPT_EXITTB);
                        /* ensure that no TB jump will be modified as
                           the program flow was changed */
                        next_tb = 0;
                    }
                }

#ifdef TARGET_PROTO_ARM_M
                if(env->regs[15] >= ARM_M_EXC_RETURN_MIN) {
                    do_v7m_exception_exit(env);
                    next_tb = 0;
                    if(automatic_sleep_after_interrupt(env)) {
                        env->exit_request = true;
                    }
                    /* Re-run interrupt arbitration after restoring the
                     * preempted execution state. */
                    continue;
                } else if(env->regs[15] >= ARM_M_FNC_RETURN_MIN) {
                    do_v7m_secure_return(env);
                    next_tb = 0;
                    continue;
                }
#endif
                if(unlikely(env->mmu_fault)) {
                    env->exception_index = MMU_EXTERNAL_FAULT;
                    cpu_loop_exit_without_hook(env);
                }

                if(unlikely(env->exception_index != -1)) {
                    cpu_loop_exit_without_hook(env);
                }
                if(cpu->instructions_count_value == cpu->instructions_count_limit) {
                    env->exit_request = 1;
                }
                if(unlikely(env->exit_request)) {
                    env->exception_index = EXCP_INTERRUPT;
                    cpu_loop_exit_without_hook(env);
                }
                if(unlikely(env->tb_restart_request)) {
                    env->tb_restart_request = 0;
                    cpu_loop_exit_without_hook(env);
                }

                tb = tb_find_fast(env);
                if(unlikely(env->exception_index != -1)) {
                    /* A bus error while reading instructions can raise a
                     * synchronous exception during translation. Do not execute
                     * or cache the block generated from the bus read's
                     * placeholder return value. */
                    tb_phys_invalidate(tb, -1);
                    cpu_loop_exit_without_hook(env);
                }
                /* Note: we do it here to avoid a gcc bug on Mac OS X when
                   doing it in tb_find_slow */
                if(tb_invalidated_flag) {
                    /* as some TB could have been invalidated because
                       of memory exceptions while generating the code, we
                       must recompute the hash index here */
                    next_tb = 0;
                    tb_invalidated_flag = 0;
                }
                /* see if we can patch the calling TB. When the TB
                   spans two pages, we cannot safely do a direct
                   jump.
                   We do not chain blocks if the chaining is explicitly disabled or if
                   there is a hook registered for the block footer. */

                if(!env->chaining_disabled && !env->block_finished_hook_present && next_tb != 0 && tb->page_addr[1] == -1) {
                    tb_add_jump((TranslationBlock *)(next_tb & ~3), next_tb & 3, tb);
                }

                /* cpu_interrupt might be called while translating the
                   TB, but before it is linked into a potentially
                   infinite loop and becomes env->current_tb. Avoid
                   starting execution if there is a pending interrupt. */
                env->current_tb = tb;
                asm volatile("" ::: "memory");
                if(likely(!env->exit_request)) {
                    tc_ptr = (uint8_t *)rw_ptr_to_rx(tb->tc_ptr);
                    /* execute the generated code */
                    next_tb = tcg_tb_exec(env, tc_ptr);
                    /* Flush the list after every unchained block */
                    flush_dirty_addresses_list();
                    if((next_tb & 3) == EXIT_TB_FORCE) {
                        tb = (TranslationBlock *)(uintptr_t)(next_tb & ~3);
                        /* Restore PC.  */
                        cpu_pc_from_tb(env, tb);
                        next_tb = 0;
                        env->exception_index = EXCP_INTERRUPT;
                        cpu_loop_exit_without_hook(env);
                    }
                }
                env->current_tb = NULL;
                /* reset soft MMU for next block (it can currently
                   only be set by a memory fault) */
            } /* for(;;) */
        } else {
            /* Reload env after longjmp - the compiler may have smashed all
             * local variables as longjmp is marked 'noreturn'. */
            env = cpu;
        }
    } /* for(;;) */

    cpu_exec_epilogue(env);

    return ret;
}

#ifndef __llvm__
#pragma GCC diagnostic pop
#endif
