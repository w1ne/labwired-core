/*
 *  Software MMU support
 *
 * Generate helpers used by TCG for qemu_ld/st ops and code load
 * functions.
 *
 * Included from target op helpers and exec.c.
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
#include "infrastructure.h"
#include <stdint.h>
#include "atomic.h"
#include "cpu.h"

extern void *global_retaddr;

#define DATA_SIZE (1 << SHIFT)

#if DATA_SIZE == 8
#define SUFFIX    q
#define USUFFIX   q
#define DATA_TYPE uint64_t
#elif DATA_SIZE == 4
#define SUFFIX    l
#define USUFFIX   l
#define DATA_TYPE uint32_t
#elif DATA_SIZE == 2
#define SUFFIX    w
#define USUFFIX   uw
#define DATA_TYPE uint16_t
#elif DATA_SIZE == 1
#define SUFFIX    b
#define USUFFIX   ub
#define DATA_TYPE uint8_t
#else
#error unsupported data size
#endif

#ifdef SOFTMMU_CODE_ACCESS
#define READ_ACCESS_TYPE ACCESS_INST_FETCH
#define ADDR_READ        addr_code
#else
#define READ_ACCESS_TYPE ACCESS_DATA_LOAD
#define ADDR_READ        addr_read
#endif

#define MEMORY_IO_READ  0
#define MEMORY_IO_WRITE 1
#define MEMORY_READ     2
#define MEMORY_WRITE    3
#define INSN_FETCH      4

#ifdef ALIGNED_ONLY
void do_unaligned_access(target_ulong addr, int is_write, int is_user, void *retaddr);
#endif

uint32_t local_ntohl(uint32_t n);

void notdirty_mem_writeb(void *opaque, target_phys_addr_t ram_addr, uint32_t val);
void notdirty_mem_writew(void *opaque, target_phys_addr_t ram_addr, uint32_t val);
void notdirty_mem_writel(void *opaque, target_phys_addr_t ram_addr, uint32_t val);

static DATA_TYPE glue(glue(slow_ld, SUFFIX), MMUSUFFIX)(target_ulong addr, int mmu_idx, void *retaddr);
static inline DATA_TYPE glue(glue(glue(slow_ld, SUFFIX), _err), MMUSUFFIX)(target_ulong addr, int mmu_idx, void *retaddr,
                                                                           int *err);
static inline DATA_TYPE glue(io_read, SUFFIX)(target_phys_addr_t physaddr, target_ulong addr, void *retaddr)
{
    DATA_TYPE res;
    physaddr = (physaddr & TARGET_PAGE_MASK) + addr;
    cpu->mem_io_pc = (uintptr_t)retaddr;
    cpu->mem_io_vaddr = addr;
    uint64_t cpustate = cpu_get_state_for_memory_transaction(env, physaddr, READ_ACCESS_TYPE);
    unlock_dangling_locks(cpu);
#if SHIFT == 0
    res = tlib_read_byte(physaddr, cpustate);
#elif SHIFT == 1
    res = tlib_read_word(physaddr, cpustate);
#elif SHIFT == 2
    res = tlib_read_double_word(physaddr, cpustate);
#else
    res = tlib_read_quad_word(physaddr, cpustate);
#endif /* SHIFT > 2 */
    return res;
}

static inline int glue(try_cached_access_read, SUFFIX)(target_ulong addr, DATA_TYPE *res)
{
    if(likely(QTAILQ_EMPTY(&cpu->read_cache))) {
        return 0;
    }

    CachedAccessDescriptor *crd;
    uint8_t crd_address_found = 0;
    if((cpu->last_crd != NULL) && (cpu->last_crd->address == addr)) {
        crd_address_found = 1;
        crd = cpu->last_crd;
    } else {
        QTAILQ_FOREACH(crd, &env->read_cache, entry) {
            if(crd->address == addr) {
                cpu->last_crd = crd;
                crd_address_found = 1;
                break;
            }
        }
    }

    if(!crd_address_found) {
        if(cpu->flush_cache_descriptors) {
            QTAILQ_FOREACH(crd, &env->read_cache, entry) {
                crd->access_count = 0;
            }
            cpu->flush_cache_descriptors = false;
        }
        return 0;
    }

    crd->access_count += 1;
    cpu->flush_cache_descriptors = true;

    if(addr == cpu->previous_io_access_read_address) {
        crd->previous_value = cpu->previous_io_access_read_value;
    }
    if(crd->access_count <= crd->not_cached_count) {
        return 0;
    }

    if((crd->cached_count > 0) && (crd->access_count == (crd->not_cached_count + crd->cached_count))) {
        crd->access_count = 0;
    }
    if((crd->cached_count == 0) || (crd->access_count < (crd->not_cached_count + crd->cached_count))) {
        *res = crd->previous_value;
        return 1;
    }
    return 0;
}

static inline int glue(try_cached_access_write, SUFFIX)(target_ulong addr, target_ulong val)
{
    if(likely(QTAILQ_EMPTY(&cpu->write_cache))) {
        return 0;
    }

    CachedAccessDescriptor *cwd;
    bool cwd_address_found = false;
    if((cpu->last_cwd != NULL) && (cpu->last_cwd->address == addr)) {
        cwd_address_found = true;
        cwd = cpu->last_cwd;
    } else {
        QTAILQ_FOREACH(cwd, &env->write_cache, entry) {
            if(cwd->address == addr) {
                cpu->last_cwd = cwd;
                cwd_address_found = true;
                break;
            }
        }
    }

    if(!cwd_address_found) {
        if(cpu->flush_cache_descriptors) {
            QTAILQ_FOREACH(cwd, &env->write_cache, entry) {
                cwd->access_count = 0;
            }
            cpu->flush_cache_descriptors = false;
        }
        return 0;
    }

    cwd->access_count += 1;
    cpu->flush_cache_descriptors = true;

    if(cwd->access_count <= cwd->not_cached_count) {
        return 0;
    }

    if((cwd->cached_count > 0) && (cwd->access_count == (cwd->not_cached_count + cwd->cached_count))) {
        cwd->access_count = 0;
    }
    if(((cwd->cached_count == 0) || (cwd->access_count < (cwd->not_cached_count + cwd->cached_count))) &&
       (cwd->previous_value == val)) {
        return 1;
    }
    cwd->previous_value = val;
    return 0;
}

/* handle all cases except unaligned access which span two pages */
__attribute__((always_inline)) inline DATA_TYPE REGPARM glue(glue(glue(__inner_ld, SUFFIX), _err),
                                                             MMUSUFFIX)(target_ulong addr, int mmu_idx, int *err, void *retaddr)
{
    DATA_TYPE res;
    int index;
    target_ulong tlb_addr;
    target_phys_addr_t ioaddr;
    uintptr_t addend;
    bool is_insn_fetch = (env->current_tb == NULL);

    /* test if there is match for unaligned or IO access */
    /* XXX: could done more in memory macro in a non portable way */
    index = (addr >> TARGET_PAGE_BITS) & (CPU_TLB_SIZE - 1);

    tlb_addr = cpu->tlb_table[mmu_idx][index].ADDR_READ;
    if(tlb_addr != -1 && (tlb_addr & TLB_ONE_SHOT) != 0) {
        //  TLB_ONE_SHOT pages should not be reused
        //  as there might be protected memory regions in them.
        //  A protected memory region does not have to fill the whole page;
        //  there might also be many memory regions defined for a single page.
        //  That's why we flush the page and force
        //  calling tlb_fill to check memory region
        //  restrictions on each access.
        tlb_flush_page(cpu, addr, true);
    }

redo:
    tlb_addr = cpu->tlb_table[mmu_idx][index].ADDR_READ & ~TLB_ONE_SHOT;

    if((addr & TARGET_PAGE_MASK) == (tlb_addr & (TARGET_PAGE_MASK | TLB_INVALID_MASK))) {
        if((tlb_addr & TLB_MMIO) == TLB_MMIO) {
            /* IO access */
            if((addr & (DATA_SIZE - 1)) != 0) {
                goto do_unaligned_access;
            }
            global_retaddr = retaddr;
            ioaddr = cpu->iotlb[mmu_idx][index];

            if(!glue(try_cached_access_read, SUFFIX)(addr, &res)) {
                res = glue(io_read, SUFFIX)(ioaddr, addr, retaddr);
                cpu->previous_io_access_read_address = addr;
                cpu->previous_io_access_read_value = res;
            }

            if(unlikely(cpu->tlib_is_on_memory_access_enabled != 0)) {
                tlib_assert(sizeof(res) <= sizeof(uint64_t));
                tlib_on_memory_access(CPU_PC(cpu), MEMORY_IO_READ, addr, DATA_SIZE, (uint64_t)res);
            }
        } else if(((addr & ~TARGET_PAGE_MASK) + DATA_SIZE - 1) >= TARGET_PAGE_SIZE) {
            /* slow unaligned access (it spans two pages or IO) */
        do_unaligned_access:
#ifdef ALIGNED_ONLY
            if(!cpu->allow_unaligned_accesses) {
                do_unaligned_access(addr, READ_ACCESS_TYPE, mmu_idx, retaddr);
            }
#endif
            res = glue(glue(glue(slow_ld, SUFFIX), _err), MMUSUFFIX)(addr, mmu_idx, retaddr, err);
            if(unlikely(cpu->tlib_is_on_memory_access_enabled != 0)) {
                tlib_assert(sizeof(res) <= sizeof(uint64_t));
                tlib_on_memory_access(CPU_PC(cpu), is_insn_fetch ? INSN_FETCH : MEMORY_READ, addr, DATA_SIZE, (uint64_t)res);
            }
        } else {
            /* unaligned/aligned access in the same page */
#ifdef ALIGNED_ONLY
            if(((addr & (DATA_SIZE - 1)) != 0) && !cpu->allow_unaligned_accesses) {
                do_unaligned_access(addr, READ_ACCESS_TYPE, mmu_idx, retaddr);
            }
#endif
            addend = cpu->tlb_table[mmu_idx][index].addend;
            res = glue(glue(ld, USUFFIX), _raw)((uint8_t *)(uintptr_t)(addr + addend));
            if(unlikely(cpu->tlib_is_on_memory_access_enabled != 0)) {
                tlib_assert(sizeof(res) <= sizeof(uint64_t));
                tlib_on_memory_access(CPU_PC(cpu), is_insn_fetch ? INSN_FETCH : MEMORY_READ, addr, DATA_SIZE, (uint64_t)res);
            }
        }
    } else {
        /* the page is not in the TLB : fill it */
#ifdef ALIGNED_ONLY
        if(((addr & (DATA_SIZE - 1)) != 0) && !cpu->allow_unaligned_accesses) {
            do_unaligned_access(addr, READ_ACCESS_TYPE, mmu_idx, retaddr);
        }
#endif
        if(!tlb_fill(cpu, addr, READ_ACCESS_TYPE, mmu_idx, retaddr, !!err, DATA_SIZE)) {
            goto redo;
        } else {
            if(err) {
                *err = 1;
            }
            res = -1;
        }
    }

    return res;
}
__attribute__((always_inline)) inline DATA_TYPE REGPARM glue(glue(glue(__ld, SUFFIX), _err), MMUSUFFIX)(target_ulong addr,
                                                                                                        int mmu_idx, int *err)
{
    //  ppc guests do a lot of loads that can't fault through this function so disable the debug check for now
#if defined(TARGET_PPC)
    void *retaddr = GETPC_INTERNAL();
#else
    void *retaddr = GETPC();
#endif
    return glue(glue(glue(__inner_ld, SUFFIX), _err), MMUSUFFIX)(addr, mmu_idx, NULL, retaddr);
}

DATA_TYPE REGPARM glue(glue(__ld, SUFFIX), MMUSUFFIX)(target_ulong addr, int mmu_idx)
{
    return glue(glue(glue(__ld, SUFFIX), _err), MMUSUFFIX)(addr, mmu_idx, NULL);
}

/* handle all unaligned cases */
static DATA_TYPE glue(glue(glue(slow_ld, SUFFIX), _err), MMUSUFFIX)(target_ulong addr, int mmu_idx, void *retaddr, int *err)
{
    DATA_TYPE res, res1, res2;
    int index, shift;
    target_phys_addr_t ioaddr;
    target_ulong tlb_addr, addr1, addr2;
    uintptr_t addend;

    index = (addr >> TARGET_PAGE_BITS) & (CPU_TLB_SIZE - 1);

    tlb_addr = cpu->tlb_table[mmu_idx][index].ADDR_READ;
    if(tlb_addr != -1 && (tlb_addr & TLB_ONE_SHOT) != 0) {
        //  TLB_ONE_SHOT pages should not be reused
        //  as there might be protected memory regions in them.
        //  A protected memory region does not have to fill the whole page;
        //  there might also be many memory regions defined for a single page.
        //  That's why we flush the page and force
        //  calling tlb_fill to check memory region
        //  restrictions on each access.
        tlb_flush_page(cpu, addr, true);
    }

redo:
    tlb_addr = cpu->tlb_table[mmu_idx][index].ADDR_READ & ~TLB_ONE_SHOT;

    if((addr & TARGET_PAGE_MASK) == (tlb_addr & (TARGET_PAGE_MASK | TLB_INVALID_MASK))) {
        if((tlb_addr & TLB_MMIO) == TLB_MMIO) {
            /* IO access */
            if((addr & (DATA_SIZE - 1)) != 0) {
                goto do_unaligned_access;
            }
            ioaddr = cpu->iotlb[mmu_idx][index];
            res = glue(io_read, SUFFIX)(ioaddr, addr, retaddr);
        } else if(((addr & ~TARGET_PAGE_MASK) + DATA_SIZE - 1) >= TARGET_PAGE_SIZE) {
        do_unaligned_access:
            /* slow unaligned access (it spans two pages) */
            addr1 = addr & ~(DATA_SIZE - 1);
            addr2 = addr1 + DATA_SIZE;
            res1 = glue(glue(glue(slow_ld, SUFFIX), _err), MMUSUFFIX)(addr1, mmu_idx, retaddr, err);
            res2 = glue(glue(glue(slow_ld, SUFFIX), _err), MMUSUFFIX)(addr2, mmu_idx, retaddr, err);
            shift = (addr & (DATA_SIZE - 1)) * 8;
#ifdef TARGET_WORDS_BIGENDIAN
            res = (res1 << shift) | (res2 >> ((DATA_SIZE * 8) - shift));
#else
            res = (res1 >> shift) | (res2 << ((DATA_SIZE * 8) - shift));
#endif
            res = (DATA_TYPE)res;
        } else {
            /* unaligned/aligned access in the same page */
            addend = cpu->tlb_table[mmu_idx][index].addend;
            res = glue(glue(ld, USUFFIX), _raw)((uint8_t *)(uintptr_t)(addr + addend));
        }
    } else {
        /* the page is not in the TLB : fill it */
        if(!tlb_fill(cpu, addr, READ_ACCESS_TYPE, mmu_idx, retaddr, err == NULL ? 0 : 1, DATA_SIZE)) {
            goto redo;
        } else {
            if(err != NULL) {
                *err = 1;
            }
            res = -1;
        }
    }
    return res;
}

static inline DATA_TYPE glue(glue(slow_ld, SUFFIX), MMUSUFFIX)(target_ulong addr, int mmu_idx, void *retaddr)
{
    return glue(glue(glue(slow_ld, SUFFIX), _err), MMUSUFFIX)(addr, mmu_idx, retaddr, NULL);
}

#ifndef SOFTMMU_CODE_ACCESS

void glue(glue(slow_st, SUFFIX), MMUSUFFIX)(target_ulong addr, DATA_TYPE val, int mmu_idx, void *retaddr);

static inline void glue(io_write, SUFFIX)(target_phys_addr_t physaddr, DATA_TYPE val, target_ulong addr, void *retaddr)
{
#if SHIFT <= 2
    int index;
    index = (physaddr >> IO_MEM_SHIFT) & (IO_MEM_NB_ENTRIES - 1);
#endif
    physaddr = (physaddr & TARGET_PAGE_MASK) + addr;
    cpu->mem_io_vaddr = addr;
    cpu->mem_io_pc = (uintptr_t)retaddr;
    uint64_t cpustate = cpu_get_state_for_memory_transaction(env, physaddr, ACCESS_DATA_STORE);
    unlock_dangling_locks(cpu);
    /* TODO: added stuff */
#if SHIFT <= 2
    if(index == IO_MEM_NOTDIRTY >> IO_MEM_SHIFT) {
        /* opaque is not used here, so we pass NULL */
        glue(notdirty_mem_write, SUFFIX)(NULL, physaddr, val);
        return;
    }
    /* TODO: added stuff ends */
#endif
#if SHIFT == 0
    tlib_write_byte(physaddr, val, cpustate);
#elif SHIFT == 1
    tlib_write_word(physaddr, val, cpustate);
#elif SHIFT == 2
    tlib_write_double_word(physaddr, val, cpustate);
#else
    tlib_write_quad_word(physaddr, val, cpustate);
#endif /* SHIFT > 2 */
}

__attribute__((always_inline)) inline void REGPARM glue(glue(__inner_st, SUFFIX), MMUSUFFIX)(target_ulong addr, DATA_TYPE val,
                                                                                             int mmu_idx, void *retaddr)
{
    target_phys_addr_t ioaddr;
    target_ulong tlb_addr;
    int index;
    uintptr_t addend;

#ifndef SUPPORTS_HST_ATOMICS
    //  Only needed for the old atomics implementation
    acquire_global_memory_lock(cpu);
    register_address_access(cpu, addr);
    release_global_memory_lock(cpu);
#endif

    index = (addr >> TARGET_PAGE_BITS) & (CPU_TLB_SIZE - 1);

    tlb_addr = cpu->tlb_table[mmu_idx][index].addr_write;
    if(tlb_addr != -1 && (tlb_addr & TLB_ONE_SHOT) != 0) {
        //  TLB_ONE_SHOT pages should not be reused
        //  as there might be protected memory regions in them.
        //  A protected memory region does not have to fill the whole page;
        //  there might also be many memory regions defined for a single page.
        //  That's why we flush the page and force
        //  calling tlb_fill to check memory region
        //  restrictions on each access.
        tlb_flush_page(cpu, addr, true);
    }

redo:
    tlb_addr = cpu->tlb_table[mmu_idx][index].addr_write & ~TLB_ONE_SHOT;

    if((addr & TARGET_PAGE_MASK) == (tlb_addr & (TARGET_PAGE_MASK | TLB_INVALID_MASK))) {
        if((tlb_addr & TLB_MMIO) == TLB_MMIO) {
            /* IO access */
            if((addr & (DATA_SIZE - 1)) != 0) {
                goto do_unaligned_access;
            }
            global_retaddr = retaddr;
            ioaddr = cpu->iotlb[mmu_idx][index];

            if(!glue(try_cached_access_write, SUFFIX)(addr, val)) {
                glue(io_write, SUFFIX)(ioaddr, val, addr, retaddr);
            }

            if(unlikely(cpu->tlib_is_on_memory_access_enabled != 0)) {
                tlib_assert(sizeof(val) <= sizeof(uint64_t));
                tlib_on_memory_access(CPU_PC(cpu), MEMORY_IO_WRITE, addr, DATA_SIZE, (uint64_t)val);
            }
        } else if(((addr & ~TARGET_PAGE_MASK) + DATA_SIZE - 1) >= TARGET_PAGE_SIZE) {
        do_unaligned_access:
#ifdef ALIGNED_ONLY
            if(!cpu->allow_unaligned_accesses) {
                do_unaligned_access(addr, 1, mmu_idx, retaddr);
            }
#endif
            glue(glue(slow_st, SUFFIX), MMUSUFFIX)(addr, val, mmu_idx, retaddr);
            if(unlikely(cpu->tlib_is_on_memory_access_enabled != 0)) {
                tlib_assert(sizeof(val) <= sizeof(uint64_t));
                tlib_on_memory_access(CPU_PC(cpu), MEMORY_WRITE, addr, DATA_SIZE, (uint64_t)val);
            }
        } else {
            /* aligned/unaligned access in the same page */
#ifdef ALIGNED_ONLY
            if(((addr & (DATA_SIZE - 1)) != 0) && !cpu->allow_unaligned_accesses) {
                do_unaligned_access(addr, 1, mmu_idx, retaddr);
            }
#endif

            addend = cpu->tlb_table[mmu_idx][index].addend;
            glue(glue(st, SUFFIX), _raw)((uint8_t *)(uintptr_t)(addr + addend), val);
            if(unlikely(cpu->tlib_is_on_memory_access_enabled != 0)) {
                tlib_assert(sizeof(val) <= sizeof(uint64_t));
                tlib_on_memory_access(CPU_PC(cpu), MEMORY_WRITE, addr, DATA_SIZE, (uint64_t)val);
            }
        }
    } else {
        /* the page is not in the TLB : fill it */
#ifdef ALIGNED_ONLY
        if(((addr & (DATA_SIZE - 1)) != 0) && !cpu->allow_unaligned_accesses) {
            do_unaligned_access(addr, 1, mmu_idx, retaddr);
        }
#endif
        tlb_fill(cpu, addr, 1, mmu_idx, retaddr, 0, DATA_SIZE);
        goto redo;
    }

    mark_tbs_containing_pc_as_dirty(addr, DATA_SIZE, 1);
}

__attribute__((always_inline)) inline void REGPARM glue(glue(__st, SUFFIX), MMUSUFFIX)(target_ulong addr, DATA_TYPE val,
                                                                                       int mmu_idx)
{
    glue(glue(__inner_st, SUFFIX), MMUSUFFIX)(addr, val, mmu_idx, GETPC());
}
/* handles all unaligned cases */
void glue(glue(slow_st, SUFFIX), MMUSUFFIX)(target_ulong addr, DATA_TYPE val, int mmu_idx, void *retaddr)
{
    target_phys_addr_t ioaddr;
    target_ulong tlb_addr;
    int index, i;
    uintptr_t addend;

    index = (addr >> TARGET_PAGE_BITS) & (CPU_TLB_SIZE - 1);

    tlb_addr = cpu->tlb_table[mmu_idx][index].addr_write;
    if(tlb_addr != -1 && (tlb_addr & TLB_ONE_SHOT) != 0) {
        //  TLB_ONE_SHOT pages should not be reused
        //  as there might be protected memory regions in them.
        //  A protected memory region does not have to fill the whole page;
        //  there might also be many memory regions defined for a single page.
        //  That's why we flush the page and force
        //  calling tlb_fill to check memory region
        //  restrictions on each access.
        tlb_flush_page(cpu, addr, true);
    }

redo:
    tlb_addr = cpu->tlb_table[mmu_idx][index].addr_write & ~TLB_ONE_SHOT;

    if((addr & TARGET_PAGE_MASK) == (tlb_addr & (TARGET_PAGE_MASK | TLB_INVALID_MASK))) {
        if((tlb_addr & TLB_MMIO) == TLB_MMIO) {
            /* IO access */
            if((addr & (DATA_SIZE - 1)) != 0) {
                goto do_unaligned_access;
            }
            ioaddr = cpu->iotlb[mmu_idx][index];
            glue(io_write, SUFFIX)(ioaddr, val, addr, retaddr);
        } else if(((addr & ~TARGET_PAGE_MASK) + DATA_SIZE - 1) >= TARGET_PAGE_SIZE) {
        do_unaligned_access:
            /* XXX: not efficient, but simple */
            /* Note: relies on the fact that tlb_fill() does not remove the
             * previous page from the TLB cache.  */
            for(i = DATA_SIZE - 1; i >= 0; i--) {
#ifdef TARGET_WORDS_BIGENDIAN
                glue(slow_stb, MMUSUFFIX)(addr + i, val >> (((DATA_SIZE - 1) * 8) - (i * 8)), mmu_idx, retaddr);
#else
                glue(slow_stb, MMUSUFFIX)(addr + i, val >> (i * 8), mmu_idx, retaddr);
#endif
            }
        } else {
            /* aligned/unaligned access in the same page */
            addend = cpu->tlb_table[mmu_idx][index].addend;
            glue(glue(st, SUFFIX), _raw)((uint8_t *)(uintptr_t)(addr + addend), val);
        }
    } else {
        /* the page is not in the TLB : fill it */
        tlb_fill(cpu, addr, 1, mmu_idx, retaddr, 0, DATA_SIZE);
        goto redo;
    }
}

#endif /* !defined(SOFTMMU_CODE_ACCESS) */

#undef READ_ACCESS_TYPE
#undef SHIFT
#undef DATA_TYPE
#undef SUFFIX
#undef USUFFIX
#undef DATA_SIZE
#undef ADDR_READ
