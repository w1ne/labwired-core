/*
 *  RISC-V emulation helpers
 *
 *  Author: Sagar Karandikar, sagark@eecs.berkeley.edu
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
#include "cpu.h"

#include "def-helper.h"
#include "cpu-common.h"
#include "arch_callbacks.h"

//  This is used as a part of MISA register, indicating the architecture width.
#if defined(TARGET_RISCV32)
#define RVXLEN ((target_ulong)1 << (TARGET_LONG_BITS - 2))
#elif defined(TARGET_RISCV64)
#define RVXLEN ((target_ulong)2 << (TARGET_LONG_BITS - 2))
#endif

void cpu_reset(CPUState *env)
{
    int32_t interrupt_mode = env->interrupt_mode;
    int32_t csr_validation_level = env->csr_validation_level;
    int privilege = env->privilege_architecture;
    target_ulong mhartid = env->mhartid;
    target_ulong misa_mask = env->misa_mask;
    target_ulong silenced_extensions = env->silenced_extensions;
    int32_t custom_instructions_count = env->custom_instructions_count;
    custom_instruction_descriptor_t custom_instructions[CPU_CUSTOM_INSTRUCTIONS_LIMIT];
    memcpy(custom_instructions, env->custom_instructions,
           sizeof(custom_instruction_descriptor_t) * CPU_CUSTOM_INSTRUCTIONS_LIMIT);
    uint64_t custom_csrs[CSRS_SLOTS];
    memcpy(custom_csrs, env->custom_csrs, sizeof(uint64_t) * CSRS_SLOTS);
    target_ulong vlenb = env->vlenb;
    target_ulong elen = env->elen;

    memset(env, 0, RESET_OFFSET);

    env->interrupt_mode = interrupt_mode;
    env->csr_validation_level = csr_validation_level;
    env->mhartid = mhartid;
    env->privilege_architecture = privilege;
    env->misa = misa_mask;
    env->misa_mask = misa_mask;
    env->silenced_extensions = silenced_extensions;
    env->priv = PRV_M;
    env->mtvec = DEFAULT_MTVEC;
    env->pc = DEFAULT_RSTVEC;
    env->exception_index = EXCP_NONE;
    env->clic_interrupt_pending = EXCP_NONE;
    env->clic_interrupt_vectored = 0;
    env->clic_interrupt_level = 0;
    env->clic_interrupt_priv = 0;
    set_default_nan_mode(1, &env->fp_status);
    set_default_mstatus();
    env->custom_instructions_count = custom_instructions_count;
    memcpy(env->custom_instructions, custom_instructions,
           sizeof(custom_instruction_descriptor_t) * CPU_CUSTOM_INSTRUCTIONS_LIMIT);
    memcpy(env->custom_csrs, custom_csrs, sizeof(uint64_t) * CSRS_SLOTS);
    env->pmp_napot_grain = -1;
    env->tb_broadcast_dirty = false;

    env->vlenb = vlenb ? vlenb : 64;
    env->elen = elen ? elen : 64;
}

int get_interrupts_in_order(target_ulong pending_interrupts, target_ulong priv)
{
    /* Interrupt shoud be taken in order:
     * External = priv | 0x1000
     * Software = priv | 0x0000
     * Timer    = priv | 0x0100
     * We should take interrupts for priority >= env->priv
     * If no interrupt should be taken returns -1 */
    int bit = EXCP_NONE;
    if(pending_interrupts & 0b1111) {
        for(int i = PRV_M; i >= priv; i--) {
            if((1 << (bit = i | 8)) & pending_interrupts) { /* external int */
                break;
            }
            if((1 << (bit = i | 0)) & pending_interrupts) { /* software int */
                break;
            }
            if((1 << (bit = i | 4)) & pending_interrupts) { /* timer int */
                break;
            }
        }
        return bit;
    } else {
        /* synchronous exceptions */
        bit = ctz64(pending_interrupts);
        return (bit >= TARGET_LONG_BITS - 1) ? EXCP_NONE : bit;
    }
}

static int riscv_clic_interrupt_pending(CPUState *env)
{
    target_ulong priv = env->priv;

    if(!cpu_in_clic_mode(env)) {
        return EXCP_NONE;
    }

    uint32_t mil = get_field(env->mintstatus, MINTSTATUS_MIL);
    uint32_t mintthresh = get_field(env->mintthresh, MINTTHRESH_TH);
    uint32_t mlevel = mintthresh > mil ? mintthresh : mil;
    /* all machine-mode interrupts are always enabled when at lower privilege levels */
    if(priv < PRV_M) {
        mlevel = 0;
    }

    uint32_t sil = get_field(env->mintstatus, MINTSTATUS_SIL);
    uint32_t sintthresh = get_field(env->sintthresh, SINTTHRESH_TH);
    uint32_t slevel = sintthresh > sil ? sintthresh : sil;
    /* all supervisor-mode interrupts are always enabled when at lower privilege levels */
    if(priv < PRV_S) {
        slevel = 0;
    }

    /* mstatus.mie, sstatus.sie affects CLIC interrupts as well, but only at the relevant privilege */
    bool mie = (priv == PRV_M && get_field(env->mstatus, MSTATUS_MIE)) || priv < PRV_M;
    bool sie = (priv == PRV_S && get_field(env->mstatus, MSTATUS_SIE)) || priv < PRV_S;

    if(sie && env->clic_interrupt_priv == PRV_S && env->interrupt_request & RISCV_CPU_INTERRUPT_CLIC &&
       env->clic_interrupt_level > slevel) {
        return env->clic_interrupt_pending;
    }

    if(mie && env->clic_interrupt_priv == PRV_M && env->interrupt_request & RISCV_CPU_INTERRUPT_CLIC &&
       env->clic_interrupt_level > mlevel) {
        return env->clic_interrupt_pending;
    }

    return EXCP_NONE;
}

/*
 * Return RISC-V IRQ number if an interrupt should be taken, else -1.
 * Used in cpu-exec.c
 *
 * Adapted from Spike's processor_t::take_interrupt()
 */
int riscv_cpu_hw_interrupts_pending(CPUState *env)
{
    target_ulong pending_interrupts = env->mip & env->mie;
    target_ulong priv = env->priv;
    target_ulong enabled_interrupts = (target_ulong)-1UL;

    if(cpu_in_clic_mode(env)) {
        return riscv_clic_interrupt_pending(env);
    }

    switch(priv) {
        /* Disable interrupts for lower privileges, if interrupt is not delegated it is for higher level */
        case PRV_M:
            pending_interrupts &= ~((IRQ_SS | IRQ_ST | IRQ_SE | env->custom_interrupts) & env->mideleg); /* fall through */
        case PRV_S:
            /* For future use, extension N not implemented yet */
            pending_interrupts &=
                ~((IRQ_US | IRQ_UT | IRQ_UE | env->custom_interrupts) & env->mideleg & env->sideleg); /* fall through */
        case PRV_U:
            break;
    }

    if(priv == PRV_M && !get_field(env->mstatus, MSTATUS_MIE)) {
        enabled_interrupts = 0;
    } else if(priv == PRV_S && !get_field(env->mstatus, MSTATUS_SIE)) {
        enabled_interrupts &= ~(env->mideleg);
    }

    enabled_interrupts &= pending_interrupts;

    if(!enabled_interrupts) {
        return EXCP_NONE;
    }

    return (env->privilege_architecture >= RISCV_PRIV1_11) ? get_interrupts_in_order(enabled_interrupts, priv)
                                                           : ctz64(enabled_interrupts);
}

/*
 * This is called in `get_physical_address()` only in the cases where we return the physical address without changes
 * such as when running in M-mode
 *
 * If the address is actually being translated then the check is performed separately
 */
static bool is_within_legal_address_space(target_ulong address, int access_type, int no_page_fault)
{
#if TARGET_LONG_BITS == 32
    return true;
#else
    uint64_t mask = (UINT64_C(1) << TARGET_PHYS_ADDR_SPACE_BITS) - 1;
    if((address & ~mask) == 0) {
        return true;
    }
    const char *action = access_type == ACCESS_INST_FETCH ? "execute code" : "access memory";
    if(!no_page_fault && access_type == ACCESS_INST_FETCH) {
        tlib_abortf("Trying to %s from an illegal address outside of the physical memory address space: 0x%" PRIx64 "\n", action,
                    address);
    } else {
        tlib_printf(LOG_LEVEL_WARNING,
                    "Trying to %s from an illegal address outside of the physical memory address space: 0x%" PRIx64 "\n", action,
                    address);
    }
    return false;
#endif
}

/* get_physical_address - get the physical address for this virtual address
 *
 * Do a page table walk to obtain the physical address corresponding to a
 * virtual address. Returns 0 if the translation was successful
 *
 * Adapted from Spike's mmu_t::translate and mmu_t::walk
 *
 */
static int get_physical_address(CPUState *env, target_phys_addr_t *physical, int *prot, target_ulong address, int access_type,
                                int mmu_idx, int no_page_fault)
{
    /* NOTE: the env->pc value visible here will not be
     * correct, but the value visible to the exception handler
     * (riscv_cpu_do_interrupt) is correct */
    int mode = mmu_idx;

    if(mode == PRV_M && access_type != ACCESS_INST_FETCH) {
        if(get_field(env->mstatus, MSTATUS_MPRV)) {
            mode = get_field(env->mstatus, MSTATUS_MPP);
        }
    }

    if(mode == PRV_M) {
        if(unlikely(!is_within_legal_address_space(address, access_type, no_page_fault))) {
            return TRANSLATE_FAIL;
        }
        *physical = address;
        *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
        return TRANSLATE_SUCCESS;
    }

    *prot = 0;

    target_ulong addr = address;
    target_ulong base;

    int levels = 0, ptidxbits = 0, ptesize = 0, vm = 0, sum = 0;
    int mxr = get_field(env->mstatus, MSTATUS_MXR);

    if(env->privilege_architecture >= RISCV_PRIV1_10) {
        base = get_field(env->satp, SATP_PPN) << PGSHIFT;
        sum = get_field(env->mstatus, MSTATUS_SUM);
        vm = get_field(env->satp, SATP_MODE);
        switch(vm) {
            case VM_1_10_SV32:
                levels = 2;
                ptidxbits = 10;
                ptesize = 4;
                break;
            case VM_1_10_SV39:
                levels = 3;
                ptidxbits = 9;
                ptesize = 8;
                break;
            case VM_1_10_SV48:
                levels = 4;
                ptidxbits = 9;
                ptesize = 8;
                break;
            case VM_1_10_SV57:
                levels = 5;
                ptidxbits = 9;
                ptesize = 8;
                break;
            case VM_1_10_MBARE:
                if(unlikely(!is_within_legal_address_space(address, access_type, no_page_fault))) {
                    return TRANSLATE_FAIL;
                }
                *physical = addr;
                *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
                return TRANSLATE_SUCCESS;
            default:
                tlib_abort("unsupported SATP_MODE value\n");
        }
    } else {
        base = env->sptbr << PGSHIFT;
        sum = !get_field(env->mstatus, MSTATUS_PUM);
        vm = get_field(env->mstatus, MSTATUS_VM);
        switch(vm) {
            case VM_1_09_SV32:
                levels = 2;
                ptidxbits = 10;
                ptesize = 4;
                break;
            case VM_1_09_SV39:
                levels = 3;
                ptidxbits = 9;
                ptesize = 8;
                break;
            case VM_1_09_SV48:
                levels = 4;
                ptidxbits = 9;
                ptesize = 8;
                break;
            case VM_1_09_MBARE:
                if(unlikely(!is_within_legal_address_space(address, access_type, no_page_fault))) {
                    return TRANSLATE_FAIL;
                }
                *physical = addr;
                *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
                return TRANSLATE_SUCCESS;
            default:
                tlib_abort("unsupported MSTATUS_VM value\n");
        }
    }

    int va_bits = PGSHIFT + levels * ptidxbits;
    target_ulong mask = (1L << (TARGET_LONG_BITS - (va_bits - 1))) - 1;
    target_ulong masked_msbs = (addr >> (va_bits - 1)) & mask;
    if(masked_msbs != 0 && masked_msbs != mask) {
        return TRANSLATE_FAIL;
    }

    int ptshift = (levels - 1) * ptidxbits;
    int i;
    for(i = 0; i < levels; i++, ptshift -= ptidxbits) {
        target_ulong idx = (addr >> (PGSHIFT + ptshift)) & ((1 << ptidxbits) - 1);

        /* check that physical address of PTE is legal */
        target_ulong pte_addr = base + idx * ptesize;
#if defined(TARGET_RISCV64)
        target_ulong pte = ldq_phys(pte_addr);
#else
        target_ulong pte = ldl_phys(pte_addr);
#endif
        target_ulong ppn = pte >> PTE_PPN_SHIFT;

        if(PTE_TABLE(pte)) { /* next level of page table */
            base = ppn << PGSHIFT;
        } else if((pte & PTE_U) && (mode == PRV_S) &&
                  (!sum || ((env->privilege_architecture >= RISCV_PRIV1_11) && access_type == ACCESS_INST_FETCH))) {
            break;
        } else if(!(pte & PTE_U) && (mode != PRV_S)) {
            break;
        } else if(!(pte & PTE_V) || (!(pte & PTE_R) && (pte & PTE_W))) {
            break;
        } else if(access_type == ACCESS_INST_FETCH  ? !(pte & PTE_X)
                  : access_type == ACCESS_DATA_LOAD ? !(pte & PTE_R) && !(mxr && (pte & PTE_X))
                                                    : !((pte & PTE_R) && (pte & PTE_W))) {
            break;
        } else {
            target_ulong updated_pte = pte | PTE_A | ((access_type == ACCESS_DATA_STORE) * PTE_D);
            if(pte != updated_pte) {
                /* set accessed and possibly dirty bits.
                   we only put it in the TLB if it has the right stuff */
#if defined(TARGET_RISCV64)
                stq_phys(pte_addr, updated_pte);
#else
                stl_phys(pte_addr, updated_pte);
#endif
            }

            /* for superpage mappings, make a fake leaf PTE for the TLB's
               benefit. */
            target_ulong vpn = addr >> PGSHIFT;
            *physical = (ppn | (vpn & ((1L << ptshift) - 1))) << PGSHIFT;

            /* we only give write permission for write accesses
             * (when we've just marked the PTE as dirty)
             * or if it was already dirty
             *
             * at this point, we assume that protection checks have occurred */
            if(pte & PTE_X) {
                *prot |= PAGE_EXEC;
            }
            if((pte & PTE_W) && (access_type == ACCESS_DATA_STORE || (pte & PTE_D))) {
                *prot |= PAGE_WRITE;
            }
            if((pte & PTE_R) || ((pte & PTE_X) && mxr)) {
                *prot |= PAGE_READ;
            }
            return TRANSLATE_SUCCESS;
        }
    }
    return TRANSLATE_FAIL;
}

static void raise_mmu_exception(CPUState *env, target_ulong address, int access_type)
{
    int page_fault_exceptions = (env->privilege_architecture >= RISCV_PRIV1_10) &&
                                get_field(env->satp, SATP_MODE) != VM_1_10_MBARE;
    int exception = 0;
    if(access_type == ACCESS_INST_FETCH) { /* inst access */
        exception = page_fault_exceptions ? RISCV_EXCP_INST_PAGE_FAULT : RISCV_EXCP_INST_ACCESS_FAULT;
        env->badaddr = address;
    } else if(access_type == ACCESS_DATA_STORE) { /* store access */
        exception = page_fault_exceptions ? RISCV_EXCP_STORE_PAGE_FAULT : RISCV_EXCP_STORE_AMO_ACCESS_FAULT;
        env->badaddr = address;
    } else if(access_type == ACCESS_DATA_LOAD) { /* load access */
        exception = page_fault_exceptions ? RISCV_EXCP_LOAD_PAGE_FAULT : RISCV_EXCP_LOAD_ACCESS_FAULT;
        env->badaddr = address;
    } else {
        tlib_abortf("Unsupported mmu exception raised: %d", access_type);
    }
#ifdef DEBUG
    tlib_printf(LOG_LEVEL_NOISY, "MMU raises an exception with id: %d, target address: 0x%x, current PC: 0x%x", exception,
                address, env->pc);
#endif
    env->exception_index = exception;
}

target_phys_addr_t cpu_get_phys_page_debug(CPUState *env, target_ulong addr)
{
    target_phys_addr_t phys_addr;
    int prot;
    int mem_idx = cpu_mmu_index(env);

    if(get_physical_address(env, &phys_addr, &prot, addr, ACCESS_DATA_LOAD, mem_idx, 0)) {
        return -1;
    }
    return phys_addr;
}

/* Transaction filtering by state is not yet implemented for this architecture.
 * This placeholder function is here to make it clear that more CPUs are expected to support this in the future. */
uint64_t cpu_get_state_for_memory_transaction(CPUState *env, target_ulong addr, int access_type)
{
    return 0;
}

/*
 * Assuming system mode, only called in arch_tlb_fill
 */
int cpu_handle_mmu_fault(CPUState *env, target_ulong address, int access_type, int mmu_idx, int access_width, int no_page_fault,
                         target_phys_addr_t *paddr)
{
    target_phys_addr_t pa = 0;
    int prot;
    int pmp_prot;
    int pmp_access_type = 1 << access_type;
    int overlapping_region_id;
    int ret = TRANSLATE_FAIL;
    target_ulong page_size = TARGET_PAGE_SIZE;

    ret = get_physical_address(env, &pa, &prot, address, access_type, mmu_idx, no_page_fault);
    pmp_prot = pmp_get_access(env, pa, access_width, access_type);
    if((pmp_access_type & pmp_prot) != pmp_access_type) {
        ret = TRANSLATE_FAIL;
    }
    if(ret == TRANSLATE_SUCCESS) {
        *paddr = pa;
        overlapping_region_id = pmp_find_overlapping(env, pa & TARGET_PAGE_MASK, TARGET_PAGE_SIZE, 0);

        //  are there any PMP regions defined for this page?
        if(overlapping_region_id != -1) {
            //  does the PMP region cover only a part of the page?
            if(env->pmp_state.addr[overlapping_region_id].sa > (pa & TARGET_PAGE_MASK) ||
               env->pmp_state.addr[overlapping_region_id].ea < (pa & TARGET_PAGE_MASK) + TARGET_PAGE_SIZE - 1) {
                //  this effectively makes the tlb page entry one-shot:
                //  thanks to this every access to this page will be verified against PMP
                page_size = access_width;
            } else {
                //  PMP region covers the entire page, we can safely propagate restrictions
                //  to the page level (PAGE_xxxx follows the same notation as PMP_xxxx)
                prot &= pmp_prot;
            }
        }

        tlb_set_page(env, address & TARGET_PAGE_MASK, pa & TARGET_PAGE_MASK, prot, mmu_idx, page_size);
    } else if(!no_page_fault && ret == TRANSLATE_FAIL) {
        raise_mmu_exception(env, address, access_type);
    }
    return ret;
}

/*
 * Handle Traps
 *
 * Adapted from Spike's processor_t::take_trap.
 *
 */
void do_interrupt(CPUState *env)
{
#ifdef DEBUG
    tlib_printf(LOG_LEVEL_NOISY, "Handling interrupt, index 0x%x, nmi_pending 0x%x", env->exception_index, env->nmi_pending);
#endif
    if(env->nmi_pending > NMI_NONE) {
        do_nmi(env);
        return;
    }

    if(env->exception_index == EXCP_NONE) {
        return;
    }

    if(env->interrupt_begin_callback_enabled) {
        tlib_on_interrupt_begin(env->exception_index);
    }

    target_ulong fixed_cause = 0;
    target_ulong bit = 0;
    uint8_t is_interrupt = 0;
    uint8_t is_in_clic_mode = cpu_in_clic_mode(env);

    if(env->exception_index & (RISCV_EXCP_INT_FLAG)) {
        /* hacky for now. the MSB (bit 63) indicates interrupt but cs->exception
           index is only 32 bits wide */
        fixed_cause = env->exception_index & RISCV_EXCP_INT_MASK;
        bit = fixed_cause;
        fixed_cause |= MCAUSE_INTERRUPT;
        is_interrupt = 1;
    } else {
        /* fixup User ECALL -> correct priv ECALL */
        if(env->exception_index == RISCV_EXCP_U_ECALL) {
            switch(env->priv) {
                case PRV_U:
                    fixed_cause = RISCV_EXCP_U_ECALL;
                    break;
                case PRV_S:
                    fixed_cause = RISCV_EXCP_S_ECALL;
                    break;
                case PRV_H:
                    fixed_cause = RISCV_EXCP_H_ECALL;
                    break;
                case PRV_M:
                    fixed_cause = RISCV_EXCP_M_ECALL;
                    break;
            }
        } else {
            fixed_cause = env->exception_index;
        }
        bit = fixed_cause;
    }

    uint8_t hasbadaddr = (fixed_cause == RISCV_EXCP_ILLEGAL_INST) || (fixed_cause == RISCV_EXCP_INST_ADDR_MIS) ||
                         (fixed_cause == RISCV_EXCP_INST_ACCESS_FAULT) || (fixed_cause == RISCV_EXCP_LOAD_ADDR_MIS) ||
                         (fixed_cause == RISCV_EXCP_STORE_AMO_ADDR_MIS) || (fixed_cause == RISCV_EXCP_LOAD_ACCESS_FAULT) ||
                         (fixed_cause == RISCV_EXCP_STORE_AMO_ACCESS_FAULT) || (fixed_cause == RISCV_EXCP_INST_PAGE_FAULT) ||
                         (fixed_cause == RISCV_EXCP_LOAD_PAGE_FAULT) || (fixed_cause == RISCV_EXCP_STORE_PAGE_FAULT);

    bool is_clic_interrupt = is_in_clic_mode && is_interrupt;

    if(fixed_cause == RISCV_EXCP_ILLEGAL_INST) {
        tlib_printf(LOG_LEVEL_WARNING, "Illegal instruction at PC=0x%" PRIx64 ": 0x%" PRIx32, env->pc, env->opcode);
    }

    target_ulong int_priv = env->priv;
    if(is_clic_interrupt) {
        int_priv = env->clic_interrupt_priv;
        tlib_clic_acknowledge_interrupt();
        if(env->clic_interrupt_vectored) {
            tlib_clic_clear_edge_interrupt();
        }
    }

    /* if in CLIC mode then only medeleg is active (interrupt privilege mode is set in the CLIC instead of mideleg) */
    bool is_delegated = (is_interrupt ? env->mideleg : env->medeleg) & (1 << bit);
    if(int_priv == PRV_M || !(is_delegated || is_clic_interrupt)) {
        /* handle the trap in M-mode */
        env->mepc = env->pc;
        if(hasbadaddr) {
            if(fixed_cause == RISCV_EXCP_ILLEGAL_INST) {
                env->mtval = env->opcode;
            } else {
                env->mtval = env->badaddr;
            }
        }

        /* Lowest 2 bits of MTVEC change mode to vectored (01) or CLIC (11) */
        if(is_in_clic_mode) {
            if(is_interrupt && env->clic_interrupt_vectored) {
                target_ulong vect_addr = env->mtvt + bit * sizeof(target_ulong);
                target_ulong vect = ldp_phys(vect_addr);
                env->pc = vect;
            } else {
                /* CLIC mode exception or shv non-vectored interrupt */
                env->pc = env->mtvec & ~MTVEC_MODE_SUBMODE;
            }
        } else if(get_field(env->mtvec, MTVEC_MODE) == MTVEC_MODE_CLINT_VECTORED && is_interrupt &&
                  env->privilege_architecture >= RISCV_PRIV1_10) {
            /* CLINT vectored mode */
            env->pc = (env->mtvec & ~MTVEC_MODE) + (fixed_cause * 4);
        } else {
            /* CLINT non-vectored mode */
            env->pc = env->mtvec & ~MTVEC_MODE;
        }

        target_ulong ms = env->mstatus;
        target_ulong mpie = (env->privilege_architecture >= RISCV_PRIV1_10) ? get_field(ms, MSTATUS_MIE)
                                                                            : get_field(ms, 1 << env->priv);
        ms = set_field(ms, MSTATUS_MPIE, mpie);
        ms = set_field(ms, MSTATUS_MIE, 0);
        ms = set_field(ms, MSTATUS_MPP, env->priv);
        csr_write_helper(env, ms, CSR_MSTATUS);

        /* Use fixed_cause as a base for mcause, it already has the interrupt bit set properly */
        target_ulong mc = fixed_cause;
        /* These extra fields are present in mcause only in CLIC mode */
        if(is_in_clic_mode) {
            mc = set_field(mc, MCAUSE_MPIL, get_field(env->mintstatus, MINTSTATUS_MIL));
            mc = set_field(mc, MCAUSE_MPIE, mpie);
            mc = set_field(mc, MCAUSE_MPP, env->priv);
            /* MINHV marks that the vector table fetch caused a fault, and mepc is set to the address of the
               table entry that caused it. This is not implemented. */
            mc = set_field(mc, MCAUSE_MINHV, 0);
        }
        csr_write_helper(env, mc, CSR_MCAUSE);

        if(is_interrupt) {
            env->mintstatus = set_field(env->mintstatus, MINTSTATUS_MIL, env->clic_interrupt_level);
        }
        riscv_set_mode(env, PRV_M);
    } else {
        /* handle the trap in S-mode */
        env->sepc = env->pc;
        if(hasbadaddr) {
            if(fixed_cause == RISCV_EXCP_ILLEGAL_INST) {
                env->stval = env->opcode;
            } else {
                env->stval = env->badaddr;
            }
        }

        /* mtvec controls CLIC mode for all privileges, lowest bit of STVEC switches CLINT vectoring for S mode */
        if(is_in_clic_mode) {
            target_ulong vect_addr = env->stvt + bit * sizeof(target_ulong);
            if(is_interrupt && env->clic_interrupt_vectored) {
                target_ulong vect = ldp_phys(vect_addr);
                env->pc = vect;
            } else {
                /* CLIC mode exception or shv non-vectored interrupt */
                env->pc = env->stvec & ~MTVEC_MODE_SUBMODE;
            }
        } else if(get_field(env->stvec, MTVEC_MODE) == MTVEC_MODE_CLINT_VECTORED && is_interrupt &&
                  (env->privilege_architecture >= RISCV_PRIV1_10)) {
            /* CLINT vectored mode */
            env->pc = (env->stvec & ~MTVEC_MODE) + (fixed_cause * 4);
        } else {
            /* CLINT non-vectored mode */
            env->pc = env->stvec & ~MTVEC_MODE;
        }

        target_ulong s = env->mstatus;
        target_ulong spie = (env->privilege_architecture >= RISCV_PRIV1_10) ? get_field(s, SSTATUS_SIE)
                                                                            : get_field(s, 1 << env->priv);
        s = set_field(s, SSTATUS_SPIE, spie);
        s = set_field(s, SSTATUS_SIE, 0);
        s = set_field(s, SSTATUS_SPP, env->priv);
        csr_write_helper(env, s, CSR_SSTATUS);
        target_ulong sc = fixed_cause;
        if(is_in_clic_mode) {
            sc = set_field(sc, SCAUSE_SPIL, get_field(env->mintstatus, MINTSTATUS_SIL));
            sc = set_field(sc, SCAUSE_SPIE, spie);
            sc = set_field(sc, SCAUSE_SPP, env->priv);
            /* SINHV marks that the vector table fetch caused a fault, and sepc is set to the address of the
               table entry that caused it. This is not implemented. */
            sc = set_field(sc, SCAUSE_SINHV, 0);
        }
        csr_write_helper(env, sc, CSR_SCAUSE);
        if(is_interrupt) {
            env->mintstatus = set_field(env->mintstatus, MINTSTATUS_SIL, env->clic_interrupt_level);
        }
        riscv_set_mode(env, PRV_S);
    }

    if(unlikely(env->guest_profiler_enabled)) {
        tlib_announce_stack_change(env->pc, STACK_FRAME_ADD);
    }
    /* TODO yield load reservation  */
    env->exception_index = EXCP_NONE; /* mark as handled */
}

void do_nmi(CPUState *env)
{
    if(env->nmi_pending == NMI_NONE) {
        return;
    }
    target_ulong s = env->mstatus;
    s = set_field(s, MSTATUS_MPIE,
                  (env->privilege_architecture >= RISCV_PRIV1_10) ? get_field(s, MSTATUS_MIE)
                                                                  : get_field(s, MSTATUS_UIE << env->priv));
    s = set_field(s, MSTATUS_MPP, env->priv); /* store current priv level */
    s = set_field(s, MSTATUS_MIE, 0);
    csr_write_helper(env, s, CSR_MSTATUS);

    riscv_set_mode(env, PRV_M);

    int32_t nmi_index = ctz64(env->nmi_pending);

    csr_write_helper(env, env->nmi_mcause[nmi_index], CSR_MCAUSE);
    env->mepc = env->pc;
    env->pc = env->nmi_address + (nmi_index << 2);

    env->nmi_pending &= ~(1 << nmi_index); /* mark this nmi as handled */
}

void tlib_arch_dispose() { }

int cpu_init(const char *cpu_model)
{
    cpu->csr_validation_level = CSR_VALIDATION_FULL;
    cpu->misa_mask = cpu->misa = RVXLEN;
    pthread_mutex_init(&cpu->mip_lock, NULL);

    cpu_reset(cpu);

    return 0;
}

//  returns 1 if the PC has already been modified by the instruction
uint32_t HELPER(handle_custom_instruction)(uint64_t id, uint64_t opcode)
{
    return tlib_handle_custom_instruction(id, opcode);
}

void HELPER(handle_post_gpr_access_hook)(uint32_t register_index, uint32_t is_write)
{
    tlib_handle_post_gpr_access_hook(register_index, is_write);
}

void HELPER(handle_pre_stack_access_hook)(uint64_t address, uint32_t width, uint32_t is_write)
{
    tlib_handle_pre_stack_access_hook(address, width, is_write);
}
