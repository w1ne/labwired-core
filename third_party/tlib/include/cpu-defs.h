/*
 * common defines for all CPUs
 *
 * Copyright (c) 2003 Fabrice Bellard
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

#include <setjmp.h>
#include <inttypes.h>
#include <signal.h>
#include "osdep.h"
#include "tlib-queue.h"
#include "tlib-alloc.h"
#include "targphys.h"
#include "infrastructure.h"
#include "atomic.h"
#include "hash-table-store-test.h"
#include "unwind.h"

/* The return address may point to the start of the next instruction.
   Subtracting one gets us the call instruction itself.  */
#if defined(__arm__)
/* Thumb return addresses have the low bit set, so we need to subtract two.
   This is still safe in ARM mode because instructions are 4 bytes.  */
#define GETPC_INTERNAL() ((void *)((uintptr_t)__builtin_return_address(0) - 2))
#else
#define GETPC_INTERNAL() ((void *)((uintptr_t)__builtin_return_address(0) - 1))
#endif

//  This check currently fails on i386/x86_64, sparc, and ppc guests so disable it
//  these guests need a rewrite of how they handle instruction fetching for it to
//  work correctly
#if defined(DEBUG) && !defined(TARGET_I386) && !defined(TARGET_SPARC)
#define GETPC()                            \
    ({                                     \
        void *pc = GETPC_INTERNAL();       \
        tlib_assert(is_ptr_in_rx_buf(pc)); \
        pc;                                \
    })
#else
#define GETPC() GETPC_INTERNAL()
#endif

#ifndef TARGET_LONG_BITS
#error TARGET_LONG_BITS must be defined before including this header
#endif

#define TARGET_LONG_SIZE (TARGET_LONG_BITS / 8)

typedef int16_t target_short __attribute__((aligned(TARGET_SHORT_ALIGNMENT)));
typedef uint16_t target_ushort __attribute__((aligned(TARGET_SHORT_ALIGNMENT)));
typedef int32_t target_int __attribute__((aligned(TARGET_INT_ALIGNMENT)));
typedef uint32_t target_uint __attribute__((aligned(TARGET_INT_ALIGNMENT)));
typedef int64_t target_llong __attribute__((aligned(TARGET_LLONG_ALIGNMENT)));
typedef uint64_t target_ullong __attribute__((aligned(TARGET_LLONG_ALIGNMENT)));
/* target_ulong is the type of a virtual address */
#if TARGET_LONG_SIZE == 4
typedef int32_t target_long __attribute__((aligned(TARGET_LONG_ALIGNMENT)));
typedef uint32_t target_ulong __attribute__((aligned(TARGET_LONG_ALIGNMENT)));
#define TARGET_ULONG_MAX UINT32_MAX
#define TARGET_FMT_lx    "%08X"
#define TARGET_FMT_ld    "%d"
#define TARGET_FMT_lu    "%u"
#elif TARGET_LONG_SIZE == 8
typedef int64_t target_long __attribute__((aligned(TARGET_LONG_ALIGNMENT)));
typedef uint64_t target_ulong __attribute__((aligned(TARGET_LONG_ALIGNMENT)));
#define TARGET_ULONG_MAX UINT64_MAX
#define TARGET_FMT_lx    "%016" PRIX64
#define TARGET_FMT_ld    "%" PRId64
#define TARGET_FMT_lu    "%" PRIu64
#else
#error TARGET_LONG_SIZE undefined
#endif

#include "disas_context_base.h"

#define HOST_LONG_SIZE (HOST_LONG_BITS / 8)

#define EXCP_INTERRUPT      0x10000 /* async interruption */
#define EXCP_WFI            0x10001 /* hlt instruction reached */
#define EXCP_DEBUG          0x10002 /* cpu stopped after a breakpoint or singlestep */
#define EXCP_WATCHPOINT     0x10004
#define EXCP_RETURN_REQUEST 0x10005
#define MMU_EXTERNAL_FAULT  0x10006 /* cpu should exit to process the external mmu handler */
#define EXCP_LOCKUP         0x10007 /* ARM M-profile Lockup state entered */

#define TB_JMP_CACHE_BITS 12
#define TB_JMP_CACHE_SIZE (1 << TB_JMP_CACHE_BITS)

#define DEFAULT_EXTERNAL_MMU_RANGE_COUNT 16

/* ARM-M specific magic PC values, which can be used for exception/secure return */
#define ARM_M_EXC_RETURN_MIN 0xffffff80  //  Value is valid for Armv8-M and below, it can decrease with newer versions
#define ARM_M_FNC_RETURN_MIN 0xfeffff00

/* Only the bottom TB_JMP_PAGE_BITS of the jump cache hash bits vary for
   addresses on the same page.  The top bits are the same.  This allows
   TLB invalidation to quickly clear a subset of the hash table.  */
#define TB_JMP_PAGE_BITS (TB_JMP_CACHE_BITS / 2)
#define TB_JMP_PAGE_SIZE (1 << TB_JMP_PAGE_BITS)
#define TB_JMP_ADDR_MASK (TB_JMP_PAGE_SIZE - 1)
#define TB_JMP_PAGE_MASK (TB_JMP_CACHE_SIZE - TB_JMP_PAGE_SIZE)

#define CPU_TLB_BITS 8
#define CPU_TLB_SIZE (1 << CPU_TLB_BITS)

#if HOST_LONG_BITS == 32 && TARGET_LONG_BITS == 32
#define CPU_TLB_ENTRY_BITS 4
#else
#define CPU_TLB_ENTRY_BITS 5
#endif

typedef struct CPUTLBEntry {
    /* bit TARGET_LONG_BITS to TARGET_PAGE_BITS : virtual address
       bit TARGET_PAGE_BITS-1..4  : Nonzero for accesses that should not
                                    go directly to ram.
       bit 3                      : indicates that the entry is invalid
       bit 2..0                   : zero
     */
    union {
        struct {
            target_ulong addr_read;
            target_ulong addr_write;
            target_ulong addr_code;
        };
        target_ulong addrs[3];
    };
    /* Addend to virtual address to get host address.  IO accesses
       use the corresponding iotlb value.  */
    uintptr_t addend;
    /* padding to get a power of two size */
    uint8_t dummy[(1 << CPU_TLB_ENTRY_BITS) -
                  (sizeof(target_ulong) * 3 + ((-sizeof(target_ulong) * 3) & (sizeof(uintptr_t) - 1)) + sizeof(uintptr_t))];
} CPUTLBEntry;

extern int CPUTLBEntry_wrong_size[sizeof(CPUTLBEntry) == (1 << CPU_TLB_ENTRY_BITS) ? 1 : -1];

#define CPU_COMMON_TLB                                                \
    /* The meaning of the MMU modes is defined in the target code. */ \
    CPUTLBEntry tlb_table[NB_MMU_MODES][CPU_TLB_SIZE];                \
    target_phys_addr_t iotlb[NB_MMU_MODES][CPU_TLB_SIZE];             \
    target_ulong tlb_flush_addr;                                      \
    target_ulong tlb_flush_mask;

typedef struct CPUBreakpoint {
    target_ulong pc;
    int flags; /* BP_* */
    QTAILQ_ENTRY(CPUBreakpoint) entry;
} CPUBreakpoint;

typedef struct CachedAccessDescriptor {
    uint64_t access_count;
    uint64_t address;
    uint64_t not_cached_count;
    uint64_t cached_count;
    uint64_t previous_value;
    QTAILQ_ENTRY(CachedAccessDescriptor) entry;
} CachedAccessDescriptor;

#define MAX_OPCODE_COUNTERS 2048
typedef struct opcode_counter_descriptor {
    uint64_t opcode;
    uint64_t mask;
    uint64_t counter;
} opcode_counter_descriptor;

typedef struct ExtMmuRange {
    uint64_t id;
    target_ulong range_start;
    target_ulong range_end;
    target_ulong addend;
    uint8_t type;
    uint8_t priv;
    bool range_end_inclusive;
} ExtMmuRange;

typedef enum ExtMmuPosition {
    EMMU_POS_NONE = 0,
    EMMU_POS_REPLACE = 1,
    EMMU_POS_BEFORE = 2,
    EMMU_POS_AFTER = 3,
} ExtMmuPosition;

typedef struct opcode_hook_mask_t {
    target_ulong mask;
    target_ulong value;
} opcode_hook_mask_t;
#define CPU_HOOKS_MASKS_LIMIT 256

enum block_interrupt_cause {
    TB_INTERRUPT_NONE = 0,
    TB_INTERRUPT_INCLUDE_LAST_INSTRUCTION = 1,
    TB_INTERRUPT_EXCLUDE_LAST_INSTRUCTION = 2,
    TB_INTERRUPT_SYNCHRONOUS_EXCEPTION = 3,
};

#define MAX_IO_ACCESS_REGIONS_COUNT 1024

#define CPU_TEMP_BUF_NLONGS    128
#define cpu_common_first_field instructions_count_limit
#define CPU_COMMON                                                             \
    /* --------------------------------------- */                              \
    /* warning: cleared by CPU reset           */                              \
    /* --------------------------------------- */                              \
    /* instruction counting is used to execute callback after given            \
       number of instructions */                                               \
    /* the types of instructions_count_* need to match the TCG-generated       \
       accesses in `gen_update_instructions_count` in translate-all.c */       \
    uint32_t instructions_count_limit;                                         \
    uint32_t instructions_count_value;                                         \
    uint32_t instructions_count_declaration;                                   \
    uint64_t instructions_count_total_value;                                   \
    /* soft mmu support */                                                     \
    /* in order to avoid passing too many arguments to the MMIO                \
       helpers, we store some rarely used information in the CPU               \
       context) */                                                             \
    uintptr_t mem_io_pc;       /* host pc at which the memory was              \
                                      accessed */                              \
    target_ulong mem_io_vaddr; /* target virtual addr at which the             \
                                     memory was accessed */                    \
    uint32_t wfi;              /* Nonzero if the CPU is in suspend state */    \
    uint32_t interrupt_request;                                                \
    volatile sig_atomic_t exit_request;                                        \
    int tb_restart_request;                                                    \
    int tb_interrupt_request_from_callback;                                    \
    int tb_interrupt_exception_from_callback;                                  \
                                                                               \
    uint32_t io_access_regions_count;                                          \
    uint64_t io_access_regions[MAX_IO_ACCESS_REGIONS_COUNT];                   \
    /* in previous run cpu_exec returned with WFI */                           \
    bool was_not_working;                                                      \
                                                                               \
    /* --------------------------------------- */                              \
    /* from this point: preserved by CPU reset */                              \
    /* --------------------------------------- */                              \
    /* Core interrupt code */                                                  \
    jmp_buf jmp_env;                                                           \
    int exception_index;                                                       \
    int nr_cores;   /* number of cores within this CPU package */              \
    int nr_threads; /* number of threads within this CPU */                    \
    bool mmu_fault;                                                            \
                                                                               \
    /* External mmu settings */                                                \
    ExtMmuPosition external_mmu_position;                                      \
    int external_mmu_window_count;                                             \
    int external_mmu_window_capacity;                                          \
    uint64_t external_mmu_window_next_id;                                      \
    bool external_mmu_windows_unsorted;                                        \
    /* user data */                                                            \
    /* chaining is enabled by default */                                       \
    int chaining_disabled;                                                     \
    /* tb cache is enabled by default */                                       \
    int tb_cache_disabled;                                                     \
    /* indicates if cpu should notify about marking tbs as dirty */            \
    bool tb_broadcast_dirty;                                                   \
    /* indicates if the block_finished hook is registered, implicitly          \
                          disabling block chaining */                          \
    int block_finished_hook_present;                                           \
    /* indicates if the block_begin hook is registered */                      \
    int block_begin_hook_present;                                              \
    int sync_pc_every_instruction_disabled;                                    \
    int cpu_wfi_state_change_hook_present;                                     \
    uint32_t millicycles_per_instruction;                                      \
    int interrupt_begin_callback_enabled;                                      \
    int interrupt_end_callback_enabled;                                        \
    int32_t tlib_is_on_memory_access_enabled;                                  \
    int allow_unaligned_accesses;                                              \
                                                                               \
    bool count_opcodes;                                                        \
    uint32_t opcode_counters_size;                                             \
    opcode_counter_descriptor opcode_counters[MAX_OPCODE_COUNTERS];            \
                                                                               \
    /* A unique, sequential id representing CPU for atomic (atomic.c)          \
       operations. It doesn't correspond to Infrastructure's cpuId */          \
    uint32_t atomic_id;                                                        \
    atomic_memory_state_t *atomic_memory_state;                                \
                                                                               \
    /* A table used to implement Hash table-based Store Test (HST)  */         \
    store_table_entry_t *store_table;                                          \
    /* How many prefix bits of an address are                                  \
       dedicated to the store table. */                                        \
    uint8_t store_table_bits;                                                  \
                                                                               \
    /* Keeps track of the currently active reservation */                      \
    target_ulong reserved_address;                                             \
    target_ulong locked_address;                                               \
    /* Some target architectures support 128 bit load/store exclusive          \
       that sometimes requires two consecutive                                 \
       locks to be held at the same time. */                                   \
    target_ulong locked_address_high;                                          \
                                                                               \
    /* STARTING FROM HERE FIELDS ARE NOT SERIALIZED */                         \
    struct TranslationBlock *current_tb; /* currently executing TB  */         \
    CPU_COMMON_TLB                                                             \
    ExtMmuRange *external_mmu_windows;                                         \
    QTAILQ_HEAD(breakpoints_head, CPUBreakpoint) breakpoints;                  \
    QTAILQ_HEAD(read_cache_head, CachedAccessDescriptor) read_cache;           \
    QTAILQ_HEAD(write_cache_head, CachedAccessDescriptor) write_cache;         \
    struct TranslationBlock *tb_jmp_cache[TB_JMP_CACHE_SIZE];                  \
    /* buffer for temporaries in the code generator */                         \
    long temp_buf[CPU_TEMP_BUF_NLONGS];                                        \
    /* when set any exception will force `cpu_exec` to finish immediately */   \
    int32_t return_on_exception;                                               \
    bool guest_profiler_enabled;                                               \
                                                                               \
    struct CachedAccessDescriptor *last_crd;                                   \
    struct CachedAccessDescriptor *last_cwd;                                   \
    uint64_t previous_io_access_read_address;                                  \
    uint64_t previous_io_access_read_value;                                    \
    bool flush_cache_descriptors;                                              \
                                                                               \
    int8_t are_pre_opcode_execution_hooks_enabled;                             \
    int32_t pre_opcode_execution_hooks_count;                                  \
    opcode_hook_mask_t pre_opcode_execution_hook_masks[CPU_HOOKS_MASKS_LIMIT]; \
                                                                               \
    int8_t are_post_opcode_execution_hooks_enabled;                            \
    int32_t post_opcode_execution_hooks_count;                                 \
    opcode_hook_mask_t post_opcode_execution_hook_masks[CPU_HOOKS_MASKS_LIMIT];

#define RESET_OFFSET offsetof(CPUState, jmp_env)

#define CPU_REGISTER_GETTER(width)                                                          \
    uint##width##_t tlib_get_register_value_##width(int reg_number)                         \
    {                                                                                       \
        uint##width##_t *ptr = get_reg_pointer_##width(reg_number);                         \
        if(ptr == NULL) {                                                                   \
            tlib_abortf("Read from undefined CPU register number %d detected", reg_number); \
        }                                                                                   \
                                                                                            \
        return *ptr;                                                                        \
    }                                                                                       \
                                                                                            \
    EXC_INT_1(uint##width##_t, tlib_get_register_value_##width, int, reg_number)

#define CPU_REGISTER_SETTER(width)                                                         \
    void tlib_set_register_value_##width(int reg_number, uint##width##_t value)            \
    {                                                                                      \
        uint##width##_t *ptr = get_reg_pointer_##width(reg_number);                        \
        if(ptr == NULL) {                                                                  \
            tlib_abortf("Write to undefined CPU register number %d detected", reg_number); \
        }                                                                                  \
                                                                                           \
        *ptr = value;                                                                      \
    }                                                                                      \
                                                                                           \
    EXC_VOID_2(tlib_set_register_value_##width, int, reg_number, uint##width##_t, value)

#define CPU_REGISTER_ACCESSOR(width) \
    CPU_REGISTER_GETTER(width)       \
    CPU_REGISTER_SETTER(width)
