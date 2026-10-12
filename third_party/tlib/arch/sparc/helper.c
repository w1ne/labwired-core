/*
 *  sparc helpers
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
#include <stdarg.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <inttypes.h>

#include "cpu.h"
#include "infrastructure.h"
#include "arch_callbacks.h"

static int cpu_sparc_find_by_name(sparc_def_t *cpu_def, const char *cpu_model);

/* Sparc MMU emulation */

/*
 * Sparc V8 Reference MMU (SRMMU)
 */
static const int access_table[8][8] = {
    { 0, 0, 0, 0, 8, 0, 12, 12 },
    { 0, 0, 0, 0, 8, 0, 0,  0  },
    { 8, 8, 0, 0, 0, 8, 12, 12 },
    { 8, 8, 0, 0, 0, 8, 0,  0  },
    { 8, 0, 8, 0, 8, 8, 12, 12 },
    { 8, 0, 8, 0, 8, 0, 8,  0  },
    { 8, 8, 8, 0, 8, 8, 12, 12 },
    { 8, 8, 8, 0, 8, 8, 8,  0  }
};

static const int perm_table[2][8] = {
    { PAGE_READ, PAGE_READ | PAGE_WRITE, PAGE_READ | PAGE_EXEC, PAGE_READ | PAGE_WRITE | PAGE_EXEC, PAGE_EXEC,
     PAGE_READ | PAGE_WRITE, PAGE_READ | PAGE_EXEC, PAGE_READ | PAGE_WRITE | PAGE_EXEC },
    {
     PAGE_READ, PAGE_READ | PAGE_WRITE,
     PAGE_READ | PAGE_EXEC,
     PAGE_READ | PAGE_WRITE | PAGE_EXEC,
     PAGE_EXEC, PAGE_READ,
     0, 0,
     }
};

static int get_physical_address(CPUState *env, target_phys_addr_t *physical, int *prot, int *access_index, target_ulong address,
                                int access_type, int mmu_idx, target_ulong *page_size)
{
    int access_perms = 0;
    target_phys_addr_t pde_ptr;
    uint32_t pde;
    int error_code = 0, is_dirty, is_user;
    uintptr_t page_offset;

    is_user = mmu_idx == MMU_USER_IDX;

    if((env->mmuregs[0] & MMU_E) == 0) { /* MMU disabled */
        *page_size = TARGET_PAGE_SIZE;
        //  Boot mode: instruction fetches are taken from PROM
        if(access_type == ACCESS_INST_FETCH && (env->mmuregs[0] & env->def->mmu_bm)) {
            *physical = env->prom_addr | (address & 0x7ffffULL);
            *prot = PAGE_READ | PAGE_EXEC;
            return TRANSLATE_SUCCESS;
        }
        *physical = address;
        *prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
        return TRANSLATE_SUCCESS;
    }

    *access_index = ((access_type & ACCESS_DATA_STORE) << 2) | (access_type & ACCESS_INST_FETCH) | (is_user ? 0 : 1);
    *physical = 0xffffffffffff0000ULL;

    /* SPARC reference MMU table walk: Context table->L1->L2->PTE */
    /* Context base + context number */
    pde_ptr = (env->mmuregs[1] << 4) + (env->mmuregs[2] << 2);
    pde = ldl_phys(pde_ptr);

    /* Ctx pde */
    switch(pde & PTE_ENTRYTYPE_MASK) {
        default:
        case 0:             /* Invalid */
            return 1 << 2;  //  TRANSLATE_FAIL
        case 2:             /* L0 PTE, maybe should not happen? */
        case 3:             /* Reserved */
            return 4 << 2;  //  TRANSLATE_FAIL
        case 1:             /* L0 PDE */
            pde_ptr = ((address >> 22) & ~3) + ((pde & ~3) << 4);
            pde = ldl_phys(pde_ptr);

            switch(pde & PTE_ENTRYTYPE_MASK) {
                default:
                case 0:                          /* Invalid */
                    return (1 << 8) | (1 << 2);  //  TRANSLATE_FAIL
                case 3:                          /* Reserved */
                    return (1 << 8) | (4 << 2);  //  TRANSLATE_FAIL
                case 1:                          /* L1 PDE */
                    pde_ptr = ((address & 0xfc0000) >> 16) + ((pde & ~3) << 4);
                    pde = ldl_phys(pde_ptr);

                    switch(pde & PTE_ENTRYTYPE_MASK) {
                        default:
                        case 0:                          /* Invalid */
                            return (2 << 8) | (1 << 2);  //  TRANSLATE_FAIL
                        case 3:                          /* Reserved */
                            return (2 << 8) | (4 << 2);  //  TRANSLATE_FAIL
                        case 1:                          /* L2 PDE */
                            pde_ptr = ((address & 0x3f000) >> 10) + ((pde & ~3) << 4);
                            pde = ldl_phys(pde_ptr);

                            switch(pde & PTE_ENTRYTYPE_MASK) {
                                default:
                                case 0:                          /* Invalid */
                                    return (3 << 8) | (1 << 2);  //  TRANSLATE_FAIL
                                case 1:                          /* PDE, should not happen */
                                case 3:                          /* Reserved */
                                    return (3 << 8) | (4 << 2);  //  TRANSLATE_FAIL
                                case 2:                          /* L3 PTE */
                                    page_offset = (address & TARGET_PAGE_MASK) & (TARGET_PAGE_SIZE - 1);
                            }
                            *page_size = TARGET_PAGE_SIZE;
                            break;
                        case 2: /* L2 PTE */
                            page_offset = address & 0x3ffff;
                            *page_size = 0x40000;
                    }
                    break;
                case 2: /* L1 PTE */
                    page_offset = address & 0xffffff;
                    *page_size = 0x1000000;
            }
    }

    /* check access */
    access_perms = (pde & PTE_ACCESS_MASK) >> PTE_ACCESS_SHIFT;
    error_code = access_table[*access_index][access_perms];
    if(error_code && !((env->mmuregs[0] & MMU_NF) && is_user)) {
        return error_code;  //  TRANSLATE_FAIL
    }

    /* update page modified and dirty bits */
    is_dirty = (access_type & ACCESS_DATA_STORE) && !(pde & PG_MODIFIED_MASK);
    if(!(pde & PG_ACCESSED_MASK) || is_dirty) {
        pde |= PG_ACCESSED_MASK;
        if(is_dirty) {
            pde |= PG_MODIFIED_MASK;
        }
        stl_phys_notdirty(pde_ptr, pde);
    }

    /* the page can be put in the TLB */
    *prot = perm_table[is_user][access_perms];
    if(!(pde & PG_MODIFIED_MASK)) {
        /* only set write access if already dirty... otherwise wait
           for dirty access */
        *prot &= ~PAGE_WRITE;
    }

    /* Even if large ptes, we map only one 4KB page in the cache to
       avoid filling it too fast */
    *physical = ((target_phys_addr_t)(pde & PTE_ADDR_MASK) << 4) + page_offset;
    return error_code;  //  TRANSLATE_FAIL
}

/* Perform address translation */
int cpu_handle_mmu_fault(CPUState *env, target_ulong address, int access_type, int mmu_idx, int is_softmmu, int no_page_fault,
                         target_phys_addr_t *out_paddr)
{
    target_phys_addr_t paddr;
    target_ulong vaddr;
    target_ulong page_size;
    int error_code = 0, prot, access_index;

    error_code = get_physical_address(env, &paddr, &prot, &access_index, address, access_type, mmu_idx, &page_size);
    if(error_code == 0) {
        *out_paddr = paddr;
        vaddr = address & TARGET_PAGE_MASK;
        paddr &= TARGET_PAGE_MASK;
        tlb_set_page(env, vaddr, paddr, prot, mmu_idx, page_size);
        return TRANSLATE_SUCCESS;
    }

    if(env->mmuregs[3]) {    /* Fault status register */
        env->mmuregs[3] = 1; /* overflow (not read before another fault) */
    }
    env->mmuregs[3] |= (access_index << 5) | error_code | 2;
    env->mmuregs[4] = address; /* Fault address register */

    if((env->mmuregs[0] & MMU_NF) || env->psret == 0) {
        //  No fault mode: if a mapping is available, just override
        //  permissions. If no mapping is available, redirect accesses to
        //  neverland. Fake/overridden mappings will be flushed when
        //  switching to normal mode.
        *out_paddr = paddr;
        vaddr = address & TARGET_PAGE_MASK;
        prot = PAGE_READ | PAGE_WRITE | PAGE_EXEC;
        tlb_set_page(env, vaddr, paddr, prot, mmu_idx, TARGET_PAGE_SIZE);
        return TRANSLATE_SUCCESS;
    } else {
        if(access_type == ACCESS_INST_FETCH) {
            env->exception_index = TT_TFAULT;
        } else {
            env->exception_index = TT_DFAULT;
        }
        return TRANSLATE_FAIL;
    }
}

target_ulong mmu_probe(CPUState *env, target_ulong address, int mmulev)
{
    target_phys_addr_t pde_ptr;
    uint32_t pde;

    /* Context base + context number */
    pde_ptr = (target_phys_addr_t)(env->mmuregs[1] << 4) + (env->mmuregs[2] << 2);
    pde = ldl_phys(pde_ptr);

    switch(pde & PTE_ENTRYTYPE_MASK) {
        default:
        case 0: /* Invalid */
        case 2: /* PTE, maybe should not happen? */
        case 3: /* Reserved */
            return 0;
        case 1: /* L1 PDE */
            if(mmulev == 3) {
                return pde;
            }
            pde_ptr = ((address >> 22) & ~3) + ((pde & ~3) << 4);
            pde = ldl_phys(pde_ptr);

            switch(pde & PTE_ENTRYTYPE_MASK) {
                default:
                case 0: /* Invalid */
                case 3: /* Reserved */
                    return 0;
                case 2: /* L1 PTE */
                    return pde;
                case 1: /* L2 PDE */
                    if(mmulev == 2) {
                        return pde;
                    }
                    pde_ptr = ((address & 0xfc0000) >> 16) + ((pde & ~3) << 4);
                    pde = ldl_phys(pde_ptr);

                    switch(pde & PTE_ENTRYTYPE_MASK) {
                        default:
                        case 0: /* Invalid */
                        case 3: /* Reserved */
                            return 0;
                        case 2: /* L2 PTE */
                            return pde;
                        case 1: /* L3 PDE */
                            if(mmulev == 1) {
                                return pde;
                            }
                            pde_ptr = ((address & 0x3f000) >> 10) + ((pde & ~3) << 4);
                            pde = ldl_phys(pde_ptr);

                            switch(pde & PTE_ENTRYTYPE_MASK) {
                                default:
                                case 0: /* Invalid */
                                case 1: /* PDE, should not happen */
                                case 3: /* Reserved */
                                    return 0;
                                case 2: /* L3 PTE */
                                    return pde;
                            }
                    }
            }
    }
    return 0;
}

static int cpu_sparc_get_phys_page(CPUState *env, target_phys_addr_t *phys, target_ulong addr, int rw, int mmu_idx)
{
    target_ulong page_size;
    int prot, access_index;

    return get_physical_address(env, phys, &prot, &access_index, addr, rw, mmu_idx, &page_size);
}

target_phys_addr_t cpu_get_phys_page_debug(CPUState *env, target_ulong addr)
{
    target_phys_addr_t phys_addr;
    int mmu_idx = cpu_mmu_index(env);

    if(cpu_sparc_get_phys_page(env, &phys_addr, addr, 2, mmu_idx) != 0) {
        if(cpu_sparc_get_phys_page(env, &phys_addr, addr, 0, mmu_idx) != 0) {
            return -1;
        }
    }
    if(cpu_get_physical_page_desc(phys_addr) == IO_MEM_UNASSIGNED) {
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

void do_interrupt(CPUState *env)
{
    target_ulong tbr_base;
    int cwp, intno = env->exception_index;
    int svt = (env->def->features & CPU_FEATURE_ASR)
                  ? !!(env->asr[1] & (0x1 << 13))  //  ASR contains regs 16-31; here we are accessing ASR17
                  : 0;

    if(env->interrupt_begin_callback_enabled) {
        tlib_on_interrupt_begin(env->exception_index);
    }

    /* Compute PSR before exposing state.  */
    if(env->cc_op != CC_OP_FLAGS) {
        cpu_get_psr(env);
    }

    if(env->psret == 0) {
        if(env->exception_index == 0x80 && env->def->features & CPU_FEATURE_TA0_SHUTDOWN) {
            tlib_on_cpu_power_down();
        } else {
            cpu_abort(env, "Trap 0x%02x while interrupts disabled, Error state", env->exception_index);
        }
        return;
    }
    env->psret = 0;
    cwp = cpu_cwp_dec(env, env->cwp - 1);
    cpu_set_cwp(env, cwp);
    env->regwptr[9] = env->pc;
    env->regwptr[10] = env->npc;
    env->psrps = env->psrs;
    env->psrs = 1;
    tbr_base = (env->tbr & TBR_BASE_MASK);
    env->tbr = tbr_base | (intno << 4);
    env->pc = svt ? tbr_base : env->tbr;
    env->npc = env->pc + 4;
    env->exception_index = -1;
    env->wfi = 0;
    /* IRQ acknowledgment */
    if((intno & ~15) == TT_EXTINT) {
        tlib_acknowledge_interrupt(intno);
    }
}

void cpu_reset(CPUState *env)
{
    env->cwp = 0;
    env->wim = 1;
    env->regwptr = env->regbase + (env->cwp * 16);
    CC_OP = CC_OP_FLAGS;
    env->psret = 0;
    env->psrs = 1;
    env->psrps = 1;
    env->mmuregs[0] &= ~(MMU_E | MMU_NF);
    env->mmuregs[0] |= env->def->mmu_bm;
    env->pc = 0;
    env->npc = env->pc + 4;
    env->cache_control = 0;
}

static int cpu_sparc_register(CPUState *env, const char *cpu_model)
{
    sparc_def_t def1, *def = &def1;

    if(cpu_sparc_find_by_name(def, cpu_model) < 0) {
        return -1;
    }

    env->def = tlib_mallocz(sizeof(*def));
    memcpy(env->def, def, sizeof(*def));
    env->version = def->iu_version;
    env->fsr = def->fpu_version;
    env->nwindows = def->nwindows;
    env->mmuregs[0] |= def->mmu_version;
    cpu_sparc_set_id(env, 0);
    env->mxccregs[7] |= def->mxcc_version;
    env->asr[1] = (1 << 8) | (def->nwindows - 1);
    return 0;
}

void tlib_arch_dispose()
{
    tlib_free(cpu->def);
}

int cpu_init(const char *cpu_model)
{
    if(cpu_sparc_register(cpu, cpu_model) < 0) {
        return -1;
    }
    cpu_reset(cpu);
    return 0;
}

void cpu_sparc_set_id(CPUState *env, unsigned int cpu)
{
    env->mxccregs[7] = ((cpu + 8) & 0xf) << 24;
}

// clang-format off

static const sparc_def_t sparc_defs[] = {
    {
        .name = "Fujitsu MB86900",
        .iu_version = 0x00 << 24, /* Impl 0, ver 0 */
        .fpu_version = 4 << 17, /* FPU version 4 (Meiko) */
        .mmu_version = 0x00 << 24, /* Impl 0, ver 0 */
        .mmu_bm = 0x00004000,
        //.mmu_ctpr_mask = 0x007ffff0,
        //.mmu_cxr_mask = 0x0000003f,
        .mmu_ctpr_mask = 0xfffffffc,
        .mmu_cxr_mask = 0x000000ff,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 7,
        .features = CPU_FEATURE_FLOAT | CPU_FEATURE_FSMULD,
    },
    {
        .name = "Fujitsu MB86904",
        .iu_version = 0x04 << 24, /* Impl 0, ver 4 */
        .fpu_version = 4 << 17, /* FPU version 4 (Meiko) */
        .mmu_version = 0x04 << 24, /* Impl 0, ver 4 */
        .mmu_bm = 0x00004000,
//        .mmu_ctpr_mask = 0x00ffffc0,
//        .mmu_cxr_mask = 0x000000ff,
        .mmu_ctpr_mask = 0xfffffffc,
        .mmu_cxr_mask = 0x000000ff,
        .mmu_sfsr_mask = 0x00016fff,
        .mmu_trcr_mask = 0x00ffffff,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "Fujitsu MB86907",
        .iu_version = 0x05 << 24, /* Impl 0, ver 5 */
        .fpu_version = 4 << 17, /* FPU version 4 (Meiko) */
        .mmu_version = 0x05 << 24, /* Impl 0, ver 5 */
        .mmu_bm = 0x00004000,
//        .mmu_ctpr_mask = 0xffffffc0,
//        .mmu_cxr_mask = 0x000000ff,
        .mmu_ctpr_mask = 0xfffffffc,
        .mmu_cxr_mask = 0x000000ff,
        .mmu_sfsr_mask = 0x00016fff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "LSI L64811",
        .iu_version = 0x10 << 24, /* Impl 1, ver 0 */
        .fpu_version = 1 << 17, /* FPU version 1 (LSI L64814) */
        .mmu_version = 0x10 << 24,
        .mmu_bm = 0x00004000,
        .mmu_ctpr_mask = 0x007ffff0,
        .mmu_cxr_mask = 0x0000003f,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_FEATURE_FLOAT | CPU_FEATURE_SWAP | CPU_FEATURE_FSQRT |
        CPU_FEATURE_FSMULD,
    },
    {
        .name = "Cypress CY7C601",
        .iu_version = 0x11 << 24, /* Impl 1, ver 1 */
        .fpu_version = 3 << 17, /* FPU version 3 (Cypress CY7C602) */
        .mmu_version = 0x10 << 24,
        .mmu_bm = 0x00004000,
        .mmu_ctpr_mask = 0x007ffff0,
        .mmu_cxr_mask = 0x0000003f,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_FEATURE_FLOAT | CPU_FEATURE_SWAP | CPU_FEATURE_FSQRT |
        CPU_FEATURE_FSMULD,
    },
    {
        .name = "Cypress CY7C611",
        .iu_version = 0x13 << 24, /* Impl 1, ver 3 */
        .fpu_version = 3 << 17, /* FPU version 3 (Cypress CY7C602) */
        .mmu_version = 0x10 << 24,
        .mmu_bm = 0x00004000,
        .mmu_ctpr_mask = 0x007ffff0,
        .mmu_cxr_mask = 0x0000003f,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_FEATURE_FLOAT | CPU_FEATURE_SWAP | CPU_FEATURE_FSQRT |
        CPU_FEATURE_FSMULD,
    },
    {
        .name = "TI MicroSparc I",
        .iu_version = 0x41000000,
        .fpu_version = 4 << 17,
        .mmu_version = 0x41000000,
        .mmu_bm = 0x00004000,
        .mmu_ctpr_mask = 0x007ffff0,
        .mmu_cxr_mask = 0x0000003f,
        .mmu_sfsr_mask = 0x00016fff,
        .mmu_trcr_mask = 0x0000003f,
        .nwindows = 7,
        .features = CPU_FEATURE_FLOAT | CPU_FEATURE_SWAP | CPU_FEATURE_MUL |
        CPU_FEATURE_DIV | CPU_FEATURE_FLUSH | CPU_FEATURE_FSQRT |
        CPU_FEATURE_FMUL,
    },
    {
        .name = "TI MicroSparc II",
        .iu_version = 0x42000000,
        .fpu_version = 4 << 17,
        .mmu_version = 0x02000000,
        .mmu_bm = 0x00004000,
        .mmu_ctpr_mask = 0x00ffffc0,
        .mmu_cxr_mask = 0x000000ff,
        .mmu_sfsr_mask = 0x00016fff,
        .mmu_trcr_mask = 0x00ffffff,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "TI MicroSparc IIep",
        .iu_version = 0x42000000,
        .fpu_version = 4 << 17,
        .mmu_version = 0x04000000,
        .mmu_bm = 0x00004000,
        .mmu_ctpr_mask = 0x00ffffc0,
        .mmu_cxr_mask = 0x000000ff,
        .mmu_sfsr_mask = 0x00016bff,
        .mmu_trcr_mask = 0x00ffffff,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "TI SuperSparc 40", // STP1020NPGA
        .iu_version = 0x41000000, // SuperSPARC 2.x
        .fpu_version = 0 << 17,
        .mmu_version = 0x00000800, // SuperSPARC 2.x, no MXCC
        .mmu_bm = 0x00002000,
        .mmu_ctpr_mask = 0xffffffc0,
        .mmu_cxr_mask = 0x0000ffff,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "TI SuperSparc 50", // STP1020PGA
        .iu_version = 0x40000000, // SuperSPARC 3.x
        .fpu_version = 0 << 17,
        .mmu_version = 0x01000800, // SuperSPARC 3.x, no MXCC
        .mmu_bm = 0x00002000,
        .mmu_ctpr_mask = 0xffffffc0,
        .mmu_cxr_mask = 0x0000ffff,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "TI SuperSparc 51",
        .iu_version = 0x40000000, // SuperSPARC 3.x
        .fpu_version = 0 << 17,
        .mmu_version = 0x01000000, // SuperSPARC 3.x, MXCC
        .mmu_bm = 0x00002000,
        .mmu_ctpr_mask = 0xffffffc0,
        .mmu_cxr_mask = 0x0000ffff,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .mxcc_version = 0x00000104,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "TI SuperSparc 60", // STP1020APGA
        .iu_version = 0x40000000, // SuperSPARC 3.x
        .fpu_version = 0 << 17,
        .mmu_version = 0x01000800, // SuperSPARC 3.x, no MXCC
        .mmu_bm = 0x00002000,
        .mmu_ctpr_mask = 0xffffffc0,
        .mmu_cxr_mask = 0x0000ffff,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "TI SuperSparc 61",
        .iu_version = 0x44000000, // SuperSPARC 3.x
        .fpu_version = 0 << 17,
        .mmu_version = 0x01000000, // SuperSPARC 3.x, MXCC
        .mmu_bm = 0x00002000,
        .mmu_ctpr_mask = 0xffffffc0,
        .mmu_cxr_mask = 0x0000ffff,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .mxcc_version = 0x00000104,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "TI SuperSparc II",
        .iu_version = 0x40000000, // SuperSPARC II 1.x
        .fpu_version = 0 << 17,
        .mmu_version = 0x08000000, // SuperSPARC II 1.x, MXCC
        .mmu_bm = 0x00002000,
        .mmu_ctpr_mask = 0xffffffc0,
        .mmu_cxr_mask = 0x0000ffff,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .mxcc_version = 0x00000104,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "Ross RT625",
        .iu_version = 0x1e000000,
        .fpu_version = 1 << 17,
        .mmu_version = 0x1e000000,
        .mmu_bm = 0x00004000,
        .mmu_ctpr_mask = 0x007ffff0,
        .mmu_cxr_mask = 0x0000003f,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "Ross RT620",
        .iu_version = 0x1f000000,
        .fpu_version = 1 << 17,
        .mmu_version = 0x1f000000,
        .mmu_bm = 0x00004000,
        .mmu_ctpr_mask = 0x007ffff0,
        .mmu_cxr_mask = 0x0000003f,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "BIT B5010",
        .iu_version = 0x20000000,
        .fpu_version = 0 << 17, /* B5010/B5110/B5120/B5210 */
        .mmu_version = 0x20000000,
        .mmu_bm = 0x00004000,
        .mmu_ctpr_mask = 0x007ffff0,
        .mmu_cxr_mask = 0x0000003f,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_FEATURE_FLOAT | CPU_FEATURE_SWAP | CPU_FEATURE_FSQRT |
        CPU_FEATURE_FSMULD,
    },
    {
        .name = "Matsushita MN10501",
        .iu_version = 0x50000000,
        .fpu_version = 0 << 17,
        .mmu_version = 0x50000000,
        .mmu_bm = 0x00004000,
        .mmu_ctpr_mask = 0x007ffff0,
        .mmu_cxr_mask = 0x0000003f,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_FEATURE_FLOAT | CPU_FEATURE_MUL | CPU_FEATURE_FSQRT |
        CPU_FEATURE_FSMULD,
    },
    {
        .name = "Weitek W8601",
        .iu_version = 0x90 << 24, /* Impl 9, ver 0 */
        .fpu_version = 3 << 17, /* FPU version 3 (Weitek WTL3170/2) */
        .mmu_version = 0x10 << 24,
        .mmu_bm = 0x00004000,
        .mmu_ctpr_mask = 0x007ffff0,
        .mmu_cxr_mask = 0x0000003f,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES,
    },
    {
        .name = "LEON2",
        .iu_version = 0xf2000000,
        .fpu_version = 4 << 17, /* FPU version 4 (Meiko) */
        .mmu_version = 0xf2000000,
        .mmu_bm = 0x00004000,
        //.mmu_ctpr_mask = 0x007ffff0,
       // .mmu_cxr_mask = 0x0000003f,
        .mmu_ctpr_mask = 0xfffffffc,
        .mmu_cxr_mask = 0x000000ff,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES | CPU_FEATURE_TA0_SHUTDOWN,
    },
    {
        .name = "LEON3",
        .iu_version = 0xf3000000,
        .fpu_version = 4 << 17, /* FPU version 4 (Meiko) */
        .mmu_version = 0xf3000000,
        .mmu_bm = 0x00000000,
        //.mmu_ctpr_mask = 0x007ffff0,
        //.mmu_cxr_mask = 0x0000003f,
        .mmu_ctpr_mask = 0xfffffffc,
        .mmu_cxr_mask = 0x000000ff,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 8,
        .features = CPU_DEFAULT_FEATURES | CPU_FEATURE_TA0_SHUTDOWN |
        CPU_FEATURE_ASR | CPU_FEATURE_CACHE_CTRL | CPU_FEATURE_CASA,
    },
    {
        .name = "GR716",
        .iu_version = 0xf3000000,
        .fpu_version = 4 << 17, /* FPU version 4 (Meiko) */
        .mmu_version = 0xf3000000,
        .mmu_bm = 0x00000000,
        //.mmu_ctpr_mask = 0x007ffff0,
        //.mmu_cxr_mask = 0x0000003f,
        .mmu_ctpr_mask = 0xfffffffc,
        .mmu_cxr_mask = 0x000000ff,
        .mmu_sfsr_mask = 0xffffffff,
        .mmu_trcr_mask = 0xffffffff,
        .nwindows = 31,
        .features = CPU_DEFAULT_FEATURES | CPU_FEATURE_TA0_SHUTDOWN |
        CPU_FEATURE_ASR | CPU_FEATURE_CACHE_CTRL | CPU_FEATURE_CASA,
    },
};

// clang-format on

static const char *const feature_name[] = {
    "float", "float128", "swap", "mul", "div", "flush", "fsqrt", "fmul", "vis1", "vis2", "fsmuld", "hypv", "cmt", "gl",
};

static void add_flagname_to_bitmaps(const char *flagname, uint32_t *features)
{
    unsigned int i;

    for(i = 0; i < ARRAY_SIZE(feature_name); i++) {
        if(feature_name[i] && !strcmp(flagname, feature_name[i])) {
            *features |= 1 << i;
            return;
        }
    }
    tlib_printf(LOG_LEVEL_WARNING, "CPU feature %s not found\n", flagname);
}

static int cpu_sparc_find_by_name(sparc_def_t *cpu_def, const char *cpu_model)
{
    unsigned int i;
    const sparc_def_t *def = NULL;
    char *s = tlib_strdup(cpu_model);
    char *featurestr, *name = strtok(s, ",");
    uint32_t plus_features = 0;
    uint32_t minus_features = 0;
    uint64_t iu_version;
    uint32_t fpu_version, mmu_version, nwindows;

    for(i = 0; i < ARRAY_SIZE(sparc_defs); i++) {
        if(strcasecmp(name, sparc_defs[i].name) == 0) {
            def = &sparc_defs[i];
        }
    }
    if(!def) {
        goto error;
    }
    memcpy(cpu_def, def, sizeof(*def));

    featurestr = strtok(NULL, ",");
    while(featurestr) {
        char *val;

        if(featurestr[0] == '+') {
            add_flagname_to_bitmaps(featurestr + 1, &plus_features);
        } else if(featurestr[0] == '-') {
            add_flagname_to_bitmaps(featurestr + 1, &minus_features);
        } else if((val = strchr(featurestr, '='))) {
            *val = 0;
            val++;
            if(!strcmp(featurestr, "iu_version")) {
                char *err;

                iu_version = strtoll(val, &err, 0);
                if(!*val || *err) {
                    tlib_printf(LOG_LEVEL_ERROR, "bad numerical value %s\n", val);
                    goto error;
                }
                cpu_def->iu_version = iu_version;
            } else if(!strcmp(featurestr, "fpu_version")) {
                char *err;

                fpu_version = strtol(val, &err, 0);
                if(!*val || *err) {
                    tlib_printf(LOG_LEVEL_ERROR, "bad numerical value %s\n", val);
                    goto error;
                }
                cpu_def->fpu_version = fpu_version;
            } else if(!strcmp(featurestr, "mmu_version")) {
                char *err;

                mmu_version = strtol(val, &err, 0);
                if(!*val || *err) {
                    tlib_printf(LOG_LEVEL_ERROR, "bad numerical value %s\n", val);
                    goto error;
                }
                cpu_def->mmu_version = mmu_version;
            } else if(!strcmp(featurestr, "nwindows")) {
                char *err;

                nwindows = strtol(val, &err, 0);
                if(!*val || *err || nwindows > MAX_NWINDOWS || nwindows < MIN_NWINDOWS) {
                    tlib_printf(LOG_LEVEL_ERROR, "bad numerical value %s\n", val);
                    goto error;
                }
                cpu_def->nwindows = nwindows;
            } else {
                tlib_printf(LOG_LEVEL_ERROR, "unrecognized feature %s\n", featurestr);
                goto error;
            }
        } else {
            tlib_printf(LOG_LEVEL_ERROR,
                        "feature string `%s' not in format "
                        "(+feature|-feature|feature=xyz)\n",
                        featurestr);
            goto error;
        }
        featurestr = strtok(NULL, ",");
    }
    cpu_def->features |= plus_features;
    cpu_def->features &= ~minus_features;
    tlib_free(s);
    return 0;

error:
    tlib_free(s);
    return -1;
}

void cpu_before_save(CPUState *env)
{
    cpu_set_cwp(env, env->cwp);
    env->psr = cpu_get_psr(env);
}

void cpu_after_load(CPUState *env)
{
    env->cwp = 0;
    cpu_put_psr(env, env->psr);
}
