/*
 *  i386 helpers (without register variable usage)
 *
 *  Copyright (c) 2003 Fabrice Bellard
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
#include <stdarg.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <inttypes.h>

#include "cpu.h"
#include "infrastructure.h"

/* NOTE: must be called outside the CPU execute loop */
void cpu_reset(CPUState *env)
{
    int i;
    /* Preserve cpuid info */

    uint32_t cpuid_level = env->cpuid_level;
    uint32_t cpuid_vendor1 = env->cpuid_vendor1;
    uint32_t cpuid_vendor2 = env->cpuid_vendor2;
    uint32_t cpuid_vendor3 = env->cpuid_vendor3;
    uint32_t cpuid_version = env->cpuid_version;
    uint32_t cpuid_features = env->cpuid_features;
    uint32_t cpuid_ext_features = env->cpuid_ext_features;
    uint32_t cpuid_xlevel = env->cpuid_xlevel;
    uint32_t cpuid_ext2_features = env->cpuid_ext2_features;
    uint32_t cpuid_ext3_features = env->cpuid_ext3_features;
    uint32_t cpuid_xlevel2 = env->cpuid_xlevel2;
    uint32_t cpuid_ext4_features = env->cpuid_ext4_features;
    uint32_t cpuid_model[CPUID_MODEL_LENGTH];
    int cpuid_vendor_override = env->cpuid_vendor_override;
    memcpy(cpuid_model, env->cpuid_model, CPUID_MODEL_LENGTH * sizeof(uint32_t));

    memset(env, 0, RESET_OFFSET);

    env->old_exception = -1;

    /* init to reset state */
    env->cpuid_level = cpuid_level;
    env->cpuid_vendor1 = cpuid_vendor1;
    env->cpuid_vendor2 = cpuid_vendor2;
    env->cpuid_vendor3 = cpuid_vendor3;
    env->cpuid_version = cpuid_version;
    env->cpuid_features = cpuid_features;
    env->cpuid_ext_features = cpuid_ext_features;
    env->cpuid_xlevel = cpuid_xlevel;
    env->cpuid_ext2_features = cpuid_ext2_features;
    env->cpuid_ext3_features = cpuid_ext3_features;
    env->cpuid_xlevel2 = cpuid_xlevel2;
    env->cpuid_ext4_features = cpuid_ext4_features;
    env->cpuid_vendor_override = cpuid_vendor_override;
    memcpy(env->cpuid_model, cpuid_model, CPUID_MODEL_LENGTH * sizeof(uint32_t));

    env->hflags |= HF_SOFTMMU_MASK;
    env->hflags2 |= HF2_GIF_MASK;

    cpu_x86_update_cr0(env, 0x60000010);
    env->a20_mask = ~0x0;
    env->smbase = 0x30000;

    env->idt.limit = 0xffff;
    env->gdt.limit = 0xffff;
    env->ldt.limit = 0xffff;
    env->ldt.flags = DESC_P_MASK | (2 << DESC_TYPE_SHIFT);
    env->tr.limit = 0xffff;
    env->tr.flags = DESC_P_MASK | (11 << DESC_TYPE_SHIFT);

    /*    cpu_x86_load_seg_cache(env, R_CS, 0xf000, 0xffff0000, 0xffff,
                               DESC_P_MASK | DESC_S_MASK | DESC_CS_MASK |
                               DESC_R_MASK | DESC_A_MASK);
        cpu_x86_load_seg_cache(env, R_DS, 0, 0, 0xffff,
                               DESC_P_MASK | DESC_S_MASK | DESC_W_MASK |
                               DESC_A_MASK);
        cpu_x86_load_seg_cache(env, R_ES, 0, 0, 0xffff,
                               DESC_P_MASK | DESC_S_MASK | DESC_W_MASK |
                               DESC_A_MASK);
        cpu_x86_load_seg_cache(env, R_SS, 0, 0, 0xffff,
                               DESC_P_MASK | DESC_S_MASK | DESC_W_MASK |
                               DESC_A_MASK);
        cpu_x86_load_seg_cache(env, R_FS, 0, 0, 0xffff,
                               DESC_P_MASK | DESC_S_MASK | DESC_W_MASK |
                               DESC_A_MASK);
        cpu_x86_load_seg_cache(env, R_GS, 0, 0, 0xffff,
                               DESC_P_MASK | DESC_S_MASK | DESC_W_MASK |
                               DESC_A_MASK);

        env->eip = 0xfff0;*/
    env->regs[R_EDX] = env->cpuid_version;

    env->eflags = 0x2;

    /* FPU init */
    for(i = 0; i < 8; i++) {
        env->fptags[i] = 1;
    }
    env->fpuc = 0x37f;

    env->mxcsr = 0x1f80;

    env->pat = 0x0007040600070406ULL;
    env->msr_ia32_misc_enable = MSR_IA32_MISC_ENABLE_DEFAULT;

    memset(env->dr, 0, sizeof(env->dr));
    env->dr[6] = DR6_FIXED_1;
    env->dr[7] = DR7_FIXED_1;
    cpu_breakpoint_remove_all(env, BP_CPU);
}

static void cpu_x86_version(CPUState *env, int *family, int *model)
{
    int cpuver = env->cpuid_version;

    if(family == NULL || model == NULL) {
        return;
    }

    *family = (cpuver >> 8) & 0x0f;
    *model = ((cpuver >> 12) & 0xf0) + ((cpuver >> 4) & 0x0f);
}

/* Broadcast MCA signal for processor version 06H_EH and above */
int cpu_x86_support_mca_broadcast(CPUState *env)
{
    int family = 0;
    int model = 0;

    cpu_x86_version(env, &family, &model);
    if((family == 6 && model >= 14) || family > 6) {
        return 1;
    }

    return 0;
}

/***********************************************************/
/* x86 mmu */
/* XXX: add PGE support */

void cpu_x86_set_a20(CPUState *env, int a20_state)
{
    a20_state = (a20_state != 0);
    if(a20_state != ((env->a20_mask >> 20) & 1)) {
        /* if the cpu is currently executing code, we must unlink it and
           all the potentially executing TB */
        cpu_interrupt(env, CPU_INTERRUPT_EXITTB);

        /* when a20 is changed, all the MMU mappings are invalid, so
           we must flush everything */
        tlb_flush(env, 1, false);
        env->a20_mask = ~(1 << 20) | (a20_state << 20);
    }
}

void cpu_x86_update_cr0(CPUState *env, uint32_t new_cr0)
{
    int pe_state;

    if((new_cr0 & (CR0_PG_MASK | CR0_WP_MASK | CR0_PE_MASK)) != (env->cr[0] & (CR0_PG_MASK | CR0_WP_MASK | CR0_PE_MASK))) {
        tlb_flush(env, 1, false);
    }

#ifdef TARGET_X86_64
    if(!(env->cr[0] & CR0_PG_MASK) && (new_cr0 & CR0_PG_MASK) && (env->efer & MSR_EFER_LME)) {
        /* enter in long mode */
        /* XXX: generate an exception */
        if(!(env->cr[4] & CR4_PAE_MASK)) {
            return;
        }
        env->efer |= MSR_EFER_LMA;
        env->hflags |= HF_LMA_MASK;
    } else if((env->cr[0] & CR0_PG_MASK) && !(new_cr0 & CR0_PG_MASK) && (env->efer & MSR_EFER_LMA)) {
        /* exit long mode */
        env->efer &= ~MSR_EFER_LMA;
        env->hflags &= ~(HF_LMA_MASK | HF_CS64_MASK);
        env->eip &= 0xffffffff;
    }
#endif
    env->cr[0] = new_cr0 | CR0_ET_MASK;

    /* update PE flag in hidden flags */
    pe_state = (env->cr[0] & CR0_PE_MASK);
    env->hflags = (env->hflags & ~HF_PE_MASK) | (pe_state << HF_PE_SHIFT);
    /* ensure that ADDSEG is always set in real mode */
    env->hflags |= ((pe_state ^ 1) << HF_ADDSEG_SHIFT);
    /* update FPU flags */
    env->hflags = (env->hflags & ~(HF_MP_MASK | HF_EM_MASK | HF_TS_MASK)) |
                  ((new_cr0 << (HF_MP_SHIFT - 1)) & (HF_MP_MASK | HF_EM_MASK | HF_TS_MASK));
}

/* XXX: in legacy PAE mode, generate a GPF if reserved bits are set in
   the PDPT */
void cpu_x86_update_cr3(CPUState *env, target_ulong new_cr3)
{
    env->cr[3] = new_cr3;
    if(env->cr[0] & CR0_PG_MASK) {
        tlb_flush(env, 0, true);
    }
}

void cpu_x86_update_cr4(CPUState *env, uint32_t new_cr4)
{
    if((new_cr4 & (CR4_PGE_MASK | CR4_PAE_MASK | CR4_PSE_MASK)) != (env->cr[4] & (CR4_PGE_MASK | CR4_PAE_MASK | CR4_PSE_MASK))) {
        tlb_flush(env, 1, true);
    }
    /* SSE handling */
    if(!(env->cpuid_features & CPUID_SSE)) {
        new_cr4 &= ~CR4_OSFXSR_MASK;
    }
    if(new_cr4 & CR4_OSFXSR_MASK) {
        env->hflags |= HF_OSFXSR_MASK;
    } else {
        env->hflags &= ~HF_OSFXSR_MASK;
    }

    env->cr[4] = new_cr4;
}

/* XXX: This value should match the one returned by CPUID
 * and in exec.c */
#if defined(TARGET_X86_64)
#define PHYS_ADDR_MASK 0xfffffff000LL
#else
#define PHYS_ADDR_MASK 0xffffff000LL
#endif

/* return value:
   -1 = cannot handle fault
   0  = nothing more to do
   1  = generate PF fault
 */
int cpu_handle_mmu_fault(CPUState *env, target_ulong addr, int access_type, int mmu_idx, int no_page_fault,
                         target_phys_addr_t *out_paddr)
{
    uint64_t ptep, pte;
    target_ulong pde_addr, pte_addr;
    int error_code, is_dirty, prot, page_size, is_write, is_user;
    target_phys_addr_t paddr;
    uint32_t page_offset;
    target_ulong vaddr, virt_addr;

    is_user = mmu_idx == MMU_USER_IDX;
    is_write = (access_type == ACCESS_DATA_STORE);

    if(!(env->cr[0] & CR0_PG_MASK)) {
        pte = addr;
        virt_addr = addr & TARGET_PAGE_MASK;
        prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
        page_size = 4096;
        goto do_mapping;
    }

    if(env->cr[4] & CR4_PAE_MASK) {
        uint64_t pde, pdpe;
        target_ulong pdpe_addr;

#ifdef TARGET_X86_64
        if(env->hflags & HF_LMA_MASK) {
            uint64_t pml4e_addr, pml4e;
            int32_t sext;

            /* test virtual address sign extension */
            sext = (int64_t)addr >> 47;
            if(sext != 0 && sext != -1) {
                env->error_code = 0;
                env->exception_index = EXCP0D_GPF;
                return TRANSLATE_SUCCESS;
            }

            pml4e_addr = ((env->cr[3] & ~0xfff) + (((addr >> 39) & 0x1ff) << 3)) & env->a20_mask;
            pml4e = ldq_phys(pml4e_addr);
            if(!(pml4e & PG_PRESENT_MASK)) {
                error_code = 0;
                goto do_fault;
            }
            if(!(env->efer & MSR_EFER_NXE) && (pml4e & PG_NX_MASK)) {
                error_code = PG_ERROR_RSVD_MASK;
                goto do_fault;
            }
            if(!(pml4e & PG_ACCESSED_MASK)) {
                pml4e |= PG_ACCESSED_MASK;
                stl_phys_notdirty(pml4e_addr, pml4e);
            }
            ptep = pml4e ^ PG_NX_MASK;
            pdpe_addr = ((pml4e & PHYS_ADDR_MASK) + (((addr >> 30) & 0x1ff) << 3)) & env->a20_mask;
            pdpe = ldq_phys(pdpe_addr);
            if(!(pdpe & PG_PRESENT_MASK)) {
                error_code = 0;
                goto do_fault;
            }
            if(!(env->efer & MSR_EFER_NXE) && (pdpe & PG_NX_MASK)) {
                error_code = PG_ERROR_RSVD_MASK;
                goto do_fault;
            }
            ptep &= pdpe ^ PG_NX_MASK;
            if(!(pdpe & PG_ACCESSED_MASK)) {
                pdpe |= PG_ACCESSED_MASK;
                stl_phys_notdirty(pdpe_addr, pdpe);
            }
        } else
#endif
        {
            /* XXX: load them when cr3 is loaded ? */
            pdpe_addr = ((env->cr[3] & ~0x1f) + ((addr >> 27) & 0x18)) & env->a20_mask;
            pdpe = ldq_phys(pdpe_addr);
            if(!(pdpe & PG_PRESENT_MASK)) {
                error_code = 0;
                goto do_fault;
            }
            ptep = PG_NX_MASK | PG_USER_MASK | PG_RW_MASK;
        }

        pde_addr = ((pdpe & PHYS_ADDR_MASK) + (((addr >> 21) & 0x1ff) << 3)) & env->a20_mask;
        pde = ldq_phys(pde_addr);
        if(!(pde & PG_PRESENT_MASK)) {
            error_code = 0;
            goto do_fault;
        }
        if(!(env->efer & MSR_EFER_NXE) && (pde & PG_NX_MASK)) {
            error_code = PG_ERROR_RSVD_MASK;
            goto do_fault;
        }
        ptep &= pde ^ PG_NX_MASK;
        if(pde & PG_PSE_MASK) {
            /* 2 MB page */
            page_size = 2048 * 1024;
            ptep ^= PG_NX_MASK;
            if((ptep & PG_NX_MASK) && access_type == ACCESS_INST_FETCH) {
                goto do_fault_protect;
            }
            if(is_user) {
                if(!(ptep & PG_USER_MASK)) {
                    goto do_fault_protect;
                }
                if(is_write && !(ptep & PG_RW_MASK)) {
                    goto do_fault_protect;
                }
            } else {
                if((env->cr[0] & CR0_WP_MASK) && is_write && !(ptep & PG_RW_MASK)) {
                    goto do_fault_protect;
                }
            }
            is_dirty = is_write && !(pde & PG_DIRTY_MASK);
            if(!(pde & PG_ACCESSED_MASK) || is_dirty) {
                pde |= PG_ACCESSED_MASK;
                if(is_dirty) {
                    pde |= PG_DIRTY_MASK;
                }
                stl_phys_notdirty(pde_addr, pde);
            }
            /* align to page_size */
            pte = pde & ((PHYS_ADDR_MASK & ~(page_size - 1)) | 0xfff);
            virt_addr = addr & ~(page_size - 1);
        } else {
            /* 4 KB page */
            if(!(pde & PG_ACCESSED_MASK)) {
                pde |= PG_ACCESSED_MASK;
                stl_phys_notdirty(pde_addr, pde);
            }
            pte_addr = ((pde & PHYS_ADDR_MASK) + (((addr >> 12) & 0x1ff) << 3)) & env->a20_mask;
            pte = ldq_phys(pte_addr);
            if(!(pte & PG_PRESENT_MASK)) {
                error_code = 0;
                goto do_fault;
            }
            if(!(env->efer & MSR_EFER_NXE) && (pte & PG_NX_MASK)) {
                error_code = PG_ERROR_RSVD_MASK;
                goto do_fault;
            }
            /* combine pde and pte nx, user and rw protections */
            ptep &= pte ^ PG_NX_MASK;
            ptep ^= PG_NX_MASK;
            if((ptep & PG_NX_MASK) && access_type == ACCESS_INST_FETCH) {
                goto do_fault_protect;
            }
            if(is_user) {
                if(!(ptep & PG_USER_MASK)) {
                    goto do_fault_protect;
                }
                if(is_write && !(ptep & PG_RW_MASK)) {
                    goto do_fault_protect;
                }
            } else {
                if((env->cr[0] & CR0_WP_MASK) && is_write && !(ptep & PG_RW_MASK)) {
                    goto do_fault_protect;
                }
            }
            is_dirty = is_write && !(pte & PG_DIRTY_MASK);
            if(!(pte & PG_ACCESSED_MASK) || is_dirty) {
                pte |= PG_ACCESSED_MASK;
                if(is_dirty) {
                    pte |= PG_DIRTY_MASK;
                }
                stl_phys_notdirty(pte_addr, pte);
            }
            page_size = 4096;
            virt_addr = addr & ~0xfff;
            pte = pte & (PHYS_ADDR_MASK | 0xfff);
        }
    } else {
        uint32_t pde;

        /* page directory entry */
        pde_addr = ((env->cr[3] & ~0xfff) + ((addr >> 20) & 0xffc)) & env->a20_mask;
        pde = ldl_phys(pde_addr);
        if(!(pde & PG_PRESENT_MASK)) {
            error_code = 0;
            goto do_fault;
        }
        /* if PSE bit is set, then we use a 4MB page */
        if((pde & PG_PSE_MASK) && (env->cr[4] & CR4_PSE_MASK)) {
            page_size = 4096 * 1024;
            if(is_user) {
                if(!(pde & PG_USER_MASK)) {
                    goto do_fault_protect;
                }
                if(is_write && !(pde & PG_RW_MASK)) {
                    goto do_fault_protect;
                }
            } else {
                if((env->cr[0] & CR0_WP_MASK) && is_write && !(pde & PG_RW_MASK)) {
                    goto do_fault_protect;
                }
            }
            is_dirty = is_write && !(pde & PG_DIRTY_MASK);
            if(!(pde & PG_ACCESSED_MASK) || is_dirty) {
                pde |= PG_ACCESSED_MASK;
                if(is_dirty) {
                    pde |= PG_DIRTY_MASK;
                }
                stl_phys_notdirty(pde_addr, pde);
            }

            pte = pde & ~((page_size - 1) & ~0xfff); /* align to page_size */
            ptep = pte;
            virt_addr = addr & ~(page_size - 1);
        } else {
            if(!(pde & PG_ACCESSED_MASK)) {
                pde |= PG_ACCESSED_MASK;
                stl_phys_notdirty(pde_addr, pde);
            }

            /* page directory entry */
            pte_addr = ((pde & ~0xfff) + ((addr >> 10) & 0xffc)) & env->a20_mask;
            pte = ldl_phys(pte_addr);
            if(!(pte & PG_PRESENT_MASK)) {
                error_code = 0;
                goto do_fault;
            }
            /* combine pde and pte user and rw protections */
            ptep = pte & pde;
            if(is_user) {
                if(!(ptep & PG_USER_MASK)) {
                    goto do_fault_protect;
                }
                if(is_write && !(ptep & PG_RW_MASK)) {
                    goto do_fault_protect;
                }
            } else {
                if((env->cr[0] & CR0_WP_MASK) && is_write && !(ptep & PG_RW_MASK)) {
                    goto do_fault_protect;
                }
            }
            is_dirty = is_write && !(pte & PG_DIRTY_MASK);
            if(!(pte & PG_ACCESSED_MASK) || is_dirty) {
                pte |= PG_ACCESSED_MASK;
                if(is_dirty) {
                    pte |= PG_DIRTY_MASK;
                }
                stl_phys_notdirty(pte_addr, pte);
            }
            page_size = 4096;
            virt_addr = addr & ~0xfff;
        }
    }
    /* the page can be put in the TLB */
    prot = PAGE_READ;
    if(!(ptep & PG_NX_MASK)) {
        prot |= PAGE_EXEC;
    }
    if(pte & PG_DIRTY_MASK) {
        /* only set write access if already dirty... otherwise wait
           for dirty access */
        if(is_user) {
            if(ptep & PG_RW_MASK) {
                prot |= PAGE_WRITE;
            }
        } else {
            if(!(env->cr[0] & CR0_WP_MASK) || (ptep & PG_RW_MASK)) {
                prot |= PAGE_WRITE;
            }
        }
    }

do_mapping:
    pte = pte & env->a20_mask;

    /* Even if 4MB pages, we map only one 4KB page in the cache to
       avoid filling it too fast */
    page_offset = (addr & TARGET_PAGE_MASK) & (page_size - 1);
    paddr = (pte & TARGET_PAGE_MASK) + page_offset;
    *out_paddr = paddr;
    vaddr = virt_addr + page_offset;
    goto set_page;
set_page:
    tlb_set_page(env, vaddr, paddr, prot, mmu_idx, page_size);
    return TRANSLATE_SUCCESS;

do_fault_protect:
    error_code = PG_ERROR_P_MASK;
do_fault:
    error_code |= (is_write << PG_ERROR_W_BIT);
    if(is_user) {
        error_code |= PG_ERROR_U_MASK;
    }
    if(access_type == ACCESS_INST_FETCH && (env->efer & MSR_EFER_NXE) && (env->cr[4] & CR4_PAE_MASK)) {
        error_code |= PG_ERROR_I_D_MASK;
    }
    if(env->intercept_exceptions & (1 << EXCP0E_PAGE)) {
        /* cr2 is not modified in case of exceptions */
        stq_phys(env->vm_vmcb + offsetof(struct vmcb, control.exit_info_2), addr);
    } else {
        env->cr[2] = addr;
    }
    env->error_code = error_code;
    env->exception_index = EXCP0E_PAGE;
    return TRANSLATE_FAIL;
}

target_phys_addr_t cpu_get_phys_page_debug(CPUState *env, target_ulong addr)
{
    target_ulong pde_addr, pte_addr;
    uint64_t pte;
    target_phys_addr_t paddr;
    uint32_t page_offset;
    int page_size;

    if(env->cr[4] & CR4_PAE_MASK) {
        target_ulong pdpe_addr;
        uint64_t pde, pdpe;

#ifdef TARGET_X86_64
        if(env->hflags & HF_LMA_MASK) {
            uint64_t pml4e_addr, pml4e;
            int32_t sext;

            /* test virtual address sign extension */
            sext = (int64_t)addr >> 47;
            if(sext != 0 && sext != -1) {
                return -1;
            }

            pml4e_addr = ((env->cr[3] & ~0xfff) + (((addr >> 39) & 0x1ff) << 3)) & env->a20_mask;
            pml4e = ldq_phys(pml4e_addr);
            if(!(pml4e & PG_PRESENT_MASK)) {
                return -1;
            }

            pdpe_addr = ((pml4e & ~0xfff) + (((addr >> 30) & 0x1ff) << 3)) & env->a20_mask;
            pdpe = ldq_phys(pdpe_addr);
            if(!(pdpe & PG_PRESENT_MASK)) {
                return -1;
            }
        } else
#endif
        {
            pdpe_addr = ((env->cr[3] & ~0x1f) + ((addr >> 27) & 0x18)) & env->a20_mask;
            pdpe = ldq_phys(pdpe_addr);
            if(!(pdpe & PG_PRESENT_MASK)) {
                return -1;
            }
        }

        pde_addr = ((pdpe & ~0xfff) + (((addr >> 21) & 0x1ff) << 3)) & env->a20_mask;
        pde = ldq_phys(pde_addr);
        if(!(pde & PG_PRESENT_MASK)) {
            return -1;
        }
        if(pde & PG_PSE_MASK) {
            /* 2 MB page */
            page_size = 2048 * 1024;
            pte = pde & ~((page_size - 1) & ~0xfff); /* align to page_size */
        } else {
            /* 4 KB page */
            pte_addr = ((pde & ~0xfff) + (((addr >> 12) & 0x1ff) << 3)) & env->a20_mask;
            page_size = 4096;
            pte = ldq_phys(pte_addr);
        }
        if(!(pte & PG_PRESENT_MASK)) {
            return -1;
        }
    } else {
        uint32_t pde;

        if(!(env->cr[0] & CR0_PG_MASK)) {
            pte = addr;
            page_size = 4096;
        } else {
            /* page directory entry */
            pde_addr = ((env->cr[3] & ~0xfff) + ((addr >> 20) & 0xffc)) & env->a20_mask;
            pde = ldl_phys(pde_addr);
            if(!(pde & PG_PRESENT_MASK)) {
                return -1;
            }
            if((pde & PG_PSE_MASK) && (env->cr[4] & CR4_PSE_MASK)) {
                pte = pde & ~0x003ff000; /* align to 4MB */
                page_size = 4096 * 1024;
            } else {
                /* page directory entry */
                pte_addr = ((pde & ~0xfff) + ((addr >> 10) & 0xffc)) & env->a20_mask;
                pte = ldl_phys(pte_addr);
                if(!(pte & PG_PRESENT_MASK)) {
                    return -1;
                }
                page_size = 4096;
            }
        }
        pte = pte & env->a20_mask;
    }

    page_offset = (addr & TARGET_PAGE_MASK) & (page_size - 1);
    paddr = (pte & TARGET_PAGE_MASK) + page_offset;
    return paddr;
}

/* Transaction filtering by state is not yet implemented for this architecture.
 * This placeholder function is here to make it clear that more CPUs are expected to support this in the future. */
uint64_t cpu_get_state_for_memory_transaction(CPUState *env, target_ulong addr, int access_type)
{
    return 0;
}

void hw_breakpoint_insert(CPUState *env, int index)
{
    int err = 0;

    if(!hw_breakpoint_type(env->dr[7], index)) {
        if(hw_breakpoint_enabled(env->dr[7], index)) {
            err = cpu_breakpoint_insert(env, env->dr[index], BP_CPU, &env->cpu_breakpoint[index]);
        }
    }
    if(err) {
        env->cpu_breakpoint[index] = NULL;
    }
}

void hw_breakpoint_remove(CPUState *env, int index)
{
    if(!env->cpu_breakpoint[index]) {
        return;
    }
    if(!hw_breakpoint_type(env->dr[7], index)) {
        if(hw_breakpoint_enabled(env->dr[7], index)) {
            cpu_breakpoint_remove_by_ref(env, env->cpu_breakpoint[index]);
        }
    }
}

int check_hw_breakpoints(CPUState *env, int force_dr6_update)
{
    target_ulong dr6;
    int reg, type;
    int hit_enabled = 0;

    dr6 = env->dr[6] & ~0xf;
    for(reg = 0; reg < 4; reg++) {
        type = hw_breakpoint_type(env->dr[7], reg);
        if((type == 0 && env->dr[reg] == env->eip)) {
            dr6 |= 1 << reg;
            if(hw_breakpoint_enabled(env->dr[7], reg)) {
                hit_enabled = 1;
            }
        }
    }
    if(hit_enabled || force_dr6_update) {
        env->dr[6] = dr6;
    }
    return hit_enabled;
}

static CPUDebugExcpHandler *prev_debug_excp_handler;

static void breakpoint_handler(CPUState *env)
{
    CPUBreakpoint *bp;

    QTAILQ_FOREACH(bp, &env->breakpoints, entry) {
        if(bp->pc == env->eip) {
            if(bp->flags & BP_CPU) {
                check_hw_breakpoints(env, 1);
                raise_exception_env(EXCP01_DB, env);
            }
            break;
        }
    }
    if(prev_debug_excp_handler) {
        prev_debug_excp_handler(env);
    }
}

typedef struct MCEInjectionParams {
    CPUState *env;
    int bank;
    uint64_t status;
    uint64_t mcg_status;
    uint64_t addr;
    uint64_t misc;
    int flags;
} MCEInjectionParams;

static void mce_init(CPUState *cenv)
{
    unsigned int bank;

    if(((cenv->cpuid_version >> 8) & 0xf) >= 6 && (cenv->cpuid_features & (CPUID_MCE | CPUID_MCA)) == (CPUID_MCE | CPUID_MCA)) {
        cenv->mcg_cap = MCE_CAP_DEF | MCE_BANKS_DEF;
        cenv->mcg_ctl = ~(uint64_t)0;
        for(bank = 0; bank < MCE_BANKS_DEF; bank++) {
            cenv->mce_banks[bank * 4] = ~(uint64_t)0;
        }
    }
}

int cpu_init(const char *cpu_model)
{
    prev_debug_excp_handler = cpu_set_debug_excp_handler(breakpoint_handler);

    if(cpu_x86_register(cpu, cpu_model) < 0) {
        return -1;
    }
    mce_init(cpu);
    cpu_reset(cpu);
    apic_init_reset(env);
    return 0;
}

void do_cpu_init(CPUState *env)
{
    int sipi = is_interrupt_pending(env, CPU_INTERRUPT_SIPI);
    uint64_t pat = env->pat;
    cpu_reset(env);
    set_interrupt_pending(env, sipi);
    env->pat = pat;
    apic_init_reset(env);
    env->wfi = !cpu_is_bsp(env);
}

void do_cpu_sipi(CPUState *env)
{
    apic_sipi(env);
}

void tlib_arch_dispose() { }
