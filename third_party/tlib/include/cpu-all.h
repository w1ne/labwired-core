/*
 * defines common to all virtual CPUs
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
#pragma once

#include <stdint.h>

#include "cpu-common.h"
#include "cpu-defs.h"

/* some important defines:
 *
 * WORDS_ALIGNED : if defined, the host cpu can only make word aligned
 * memory accesses.
 *
 * HOST_WORDS_BIGENDIAN : if defined, the host cpu is big endian and
 * otherwise little endian.
 *
 * (TARGET_WORDS_ALIGNED : same for target cpu (not supported yet))
 *
 * TARGET_WORDS_BIGENDIAN : same for target cpu
 */

#ifdef HOST_WORDS_BIGENDIAN
#warning Big-Endian Host architecture is currently only partially supported, emulation errors might arise
#endif

#include "softfloat-2.h"

typedef union {
    float32 f;
    uint32_t l;
} CPU_FloatU;

/* NOTE: arm FPA is horrible as double 32 bit words are stored in big
   endian ! */
typedef union {
    float64 d;
#if defined(HOST_WORDS_BIGENDIAN)
    struct {
        uint32_t upper;
        uint32_t lower;
    } l;
#else
    struct {
        uint32_t lower;
        uint32_t upper;
    } l;
#endif
    uint64_t ll;
} CPU_DoubleU;

typedef union {
    floatx80 d;
    struct {
        uint64_t lower;
        uint16_t upper;
    } l;
} CPU_LDoubleU;

typedef union {
    float128 q;
#if defined(HOST_WORDS_BIGENDIAN)
    struct {
        uint32_t upmost;
        uint32_t upper;
        uint32_t lower;
        uint32_t lowest;
    } l;
    struct {
        uint64_t upper;
        uint64_t lower;
    } ll;
#else
    struct {
        uint32_t lowest;
        uint32_t lower;
        uint32_t upper;
        uint32_t upmost;
    } l;
    struct {
        uint64_t lower;
        uint64_t upper;
    } ll;
#endif
} CPU_QuadU;

/* CPU memory access without any memory or io remapping */

/*
 * the generic syntax for the memory accesses is:
 *
 * load: ld{type}{sign}{size}{endian}_{access_type}(ptr)
 *
 * store: st{type}{size}{endian}_{access_type}(ptr, val)
 *
 * type is:
 * (empty): integer access
 *   f    : float access
 *
 * sign is:
 * (empty): for floats or 32 bit size
 *   u    : unsigned
 *   s    : signed
 *
 * size is:
 *   b: 8 bits
 *   w: 16 bits
 *   l: 32 bits
 *   q: 64 bits
 *
 * endian is:
 * (empty): target cpu endianness or 8 bit access
 *   r    : reversed target cpu endianness (not implemented yet)
 *   be   : big endian (not implemented yet)
 *   le   : little endian (not implemented yet)
 *
 * access_type is:
 *   raw    : host memory access
 *   user   : user mode access using soft MMU
 *   kernel : kernel mode access using soft MMU
 */
static inline int ldub_p(const void *ptr)
{
    return *(uint8_t *)ptr;
}

static inline int ldsb_p(const void *ptr)
{
    return *(int8_t *)ptr;
}

static inline void stb_p(void *ptr, int v)
{
    *(uint8_t *)ptr = v;
}

/* NOTE: on arm, putting 2 in /proc/sys/debug/alignment so that the
   kernel handles unaligned load/stores may give better results, but
   it is a system wide setting : bad */
#if defined(HOST_WORDS_BIGENDIAN) || defined(WORDS_ALIGNED)

/* conservative code for little endian unaligned accesses */
static inline int lduw_le_p(const void *ptr)
{
    const uint8_t *p = ptr;
    return p[0] | (p[1] << 8);
}

static inline int ldl_le_p(const void *ptr)
{
    const uint8_t *p = ptr;
    return p[0] | (p[1] << 8) | (p[2] << 16) | (p[3] << 24);
}

static inline uint64_t ldq_le_p(const void *ptr)
{
    const uint8_t *p = ptr;
    uint32_t v1, v2;
    v1 = ldl_le_p(p);
    v2 = ldl_le_p(p + 4);
    return v1 | ((uint64_t)v2 << 32);
}

static inline void stw_le_p(void *ptr, int v)
{
    uint8_t *p = ptr;
    p[0] = v;
    p[1] = v >> 8;
}

static inline void stl_le_p(void *ptr, int v)
{
    uint8_t *p = ptr;
    p[0] = v;
    p[1] = v >> 8;
    p[2] = v >> 16;
    p[3] = v >> 24;
}

static inline void stq_le_p(void *ptr, uint64_t v)
{
    uint8_t *p = ptr;
    stl_le_p(p, (uint32_t)v);
    stl_le_p(p + 4, v >> 32);
}

/* float access */

static inline float64 ldfq_le_p(const void *ptr)
{
    CPU_DoubleU u;
    u.l.lower = ldl_le_p(ptr);
    u.l.upper = ldl_le_p(ptr + 4);
    return u.d;
}

static inline void stfq_le_p(void *ptr, float64 v)
{
    CPU_DoubleU u;
    u.d = v;
    stl_le_p(ptr, u.l.lower);
    stl_le_p(ptr + 4, u.l.upper);
}

#else

static inline int ldsw_le_p(const void *ptr)
{
    const uint8_t *p = ptr;
    return (int16_t)(p[0] | (p[1] << 8));
}

static inline int lduw_le_p(const void *ptr)
{
    return *(uint16_t *)ptr;
}

static inline int ldl_le_p(const void *ptr)
{
    return *(uint32_t *)ptr;
}

static inline uint64_t ldq_le_p(const void *ptr)
{
    return *(uint64_t *)ptr;
}

static inline void stw_le_p(void *ptr, int v)
{
    *(uint16_t *)ptr = v;
}

static inline void stl_le_p(void *ptr, int v)
{
    *(uint32_t *)ptr = v;
}

static inline void stq_le_p(void *ptr, uint64_t v)
{
    *(uint64_t *)ptr = v;
}

/* float access */

static inline float64 ldfq_le_p(const void *ptr)
{
    return *(float64 *)ptr;
}

static inline void stfq_le_p(void *ptr, float64 v)
{
    *(float64 *)ptr = v;
}
#endif

#if !defined(HOST_WORDS_BIGENDIAN) || defined(WORDS_ALIGNED)

static inline int lduw_be_p(const void *ptr)
{
#if defined(__i386__)
    int val;
    asm volatile("movzwl %1, %0\n"
                 "xchgb %b0, %h0\n"
                 : "=q"(val)
                 : "m"(*(uint16_t *)ptr));
    return val;
#else
    const uint8_t *b = ptr;
    return ((b[0] << 8) | b[1]);
#endif
}

static inline int ldsw_be_p(const void *ptr)
{
#if defined(__i386__)
    int val;
    asm volatile("movzwl %1, %0\n"
                 "xchgb %b0, %h0\n"
                 : "=q"(val)
                 : "m"(*(uint16_t *)ptr));
    return (int16_t)val;
#else
    const uint8_t *b = ptr;
    return (int16_t)((b[0] << 8) | b[1]);
#endif
}

static inline int ldl_be_p(const void *ptr)
{
#if defined(__i386__) || defined(__x86_64__)
    int val;
    asm volatile("movl %1, %0\n"
                 "bswap %0\n"
                 : "=r"(val)
                 : "m"(*(uint32_t *)ptr));
    return val;
#else
    const uint8_t *b = ptr;
    return (b[0] << 24) | (b[1] << 16) | (b[2] << 8) | b[3];
#endif
}

static inline uint64_t ldq_be_p(const void *ptr)
{
    uint32_t a, b;
    a = ldl_be_p(ptr);
    b = ldl_be_p((uint8_t *)ptr + 4);
    return (((uint64_t)a << 32) | b);
}

static inline void stw_be_p(void *ptr, int v)
{
#if defined(__i386__)
    asm volatile("xchgb %b0, %h0\n"
                 "movw %w0, %1\n"
                 : "=q"(v)
                 : "m"(*(uint16_t *)ptr), "0"(v));
#else
    uint8_t *d = (uint8_t *)ptr;
    d[0] = v >> 8;
    d[1] = v;
#endif
}

static inline void stl_be_p(void *ptr, int v)
{
#if defined(__i386__) || defined(__x86_64__)
    asm volatile("bswap %0\n"
                 "movl %0, %1\n"
                 : "=r"(v)
                 : "m"(*(uint32_t *)ptr), "0"(v));
#else
    uint8_t *d = (uint8_t *)ptr;
    d[0] = v >> 24;
    d[1] = v >> 16;
    d[2] = v >> 8;
    d[3] = v;
#endif
}

static inline void stq_be_p(void *ptr, uint64_t v)
{
    stl_be_p(ptr, v >> 32);
    stl_be_p((uint8_t *)ptr + 4, v);
}

/* float access */

static inline float64 ldfq_be_p(const void *ptr)
{
    CPU_DoubleU u;
    u.l.upper = ldl_be_p(ptr);
    u.l.lower = ldl_be_p((uint8_t *)ptr + 4);
    return u.d;
}

static inline void stfq_be_p(void *ptr, float64 v)
{
    CPU_DoubleU u;
    u.d = v;
    stl_be_p(ptr, u.l.upper);
    stl_be_p((uint8_t *)ptr + 4, u.l.lower);
}

#else

static inline int lduw_be_p(const void *ptr)
{
    return *(uint16_t *)ptr;
}

static inline int ldsw_be_p(const void *ptr)
{
    return *(int16_t *)ptr;
}

static inline int ldl_be_p(const void *ptr)
{
    return *(uint32_t *)ptr;
}

static inline uint64_t ldq_be_p(const void *ptr)
{
    return *(uint64_t *)ptr;
}

static inline void stw_be_p(void *ptr, int v)
{
    *(uint16_t *)ptr = v;
}

static inline void stl_be_p(void *ptr, int v)
{
    *(uint32_t *)ptr = v;
}

static inline void stq_be_p(void *ptr, uint64_t v)
{
    *(uint64_t *)ptr = v;
}

/* float access */

static inline float64 ldfq_be_p(const void *ptr)
{
    return *(float64 *)ptr;
}

static inline void stfq_be_p(void *ptr, float64 v)
{
    *(float64 *)ptr = v;
}

#endif

/* target CPU memory access functions */
#if defined(TARGET_WORDS_BIGENDIAN)
#define lduw_p(p)    lduw_be_p(p)
#define ldsw_p(p)    ldsw_be_p(p)
#define ldl_p(p)     ldl_be_p(p)
#define ldq_p(p)     ldq_be_p(p)
#define ldfq_p(p)    ldfq_be_p(p)
#define stw_p(p, v)  stw_be_p(p, v)
#define stl_p(p, v)  stl_be_p(p, v)
#define stq_p(p, v)  stq_be_p(p, v)
#define stfq_p(p, v) stfq_be_p(p, v)
#else
#define lduw_p(p)    lduw_le_p(p)
#define ldsw_p(p)    ldsw_le_p(p)
#define ldl_p(p)     ldl_le_p(p)
#define ldq_p(p)     ldq_le_p(p)
#define ldfq_p(p)    ldfq_le_p(p)
#define stw_p(p, v)  stw_le_p(p, v)
#define stl_p(p, v)  stl_le_p(p, v)
#define stq_p(p, v)  stq_le_p(p, v)
#define stfq_p(p, v) stfq_le_p(p, v)
#endif

/* MMU memory access macros */

/* NOTE: we use double casts if pointers and target_ulong have
   different sizes */
#define saddr(x) (uint8_t *)(uintptr_t)(x)
#define laddr(x) (uint8_t *)(uintptr_t)(x)

#define ldub_raw(p)    ldub_p(laddr((p)))
#define ldsb_raw(p)    ldsb_p(laddr((p)))
#define lduw_raw(p)    lduw_p(laddr((p)))
#define ldl_raw(p)     ldl_p(laddr((p)))
#define ldsw_raw(p)    ldsw_p(laddr((p)))
#define ldq_raw(p)     ldq_p(laddr((p)))
#define stb_raw(p, v)  stb_p(saddr((p)), v)
#define stw_raw(p, v)  stw_p(saddr((p)), v)
#define stl_raw(p, v)  stl_p(saddr((p)), v)
#define stq_raw(p, v)  stq_p(saddr((p)), v)
#define stfq_raw(p, v) stfq_p(saddr((p)), v)

/* page related stuff */

#define TARGET_PAGE_SIZE (1ull << TARGET_PAGE_BITS)
#define TARGET_PAGE_MASK ~(TARGET_PAGE_SIZE - 1)

/* ??? These should be the larger of unsigned long and target_ulong.  */
extern uintptr_t tlib_real_host_page_size;
extern uintptr_t tlib_host_page_bits;
extern uintptr_t tlib_host_page_size;
extern uintptr_t tlib_host_page_mask;

/* Access types as used in the get_phys_addr* functions */
#define ACCESS_DATA_LOAD  0
#define ACCESS_DATA_STORE 1
#define ACCESS_INST_FETCH 2
/* Helper macro for debugging purposes */
#define ACCESS_TYPE_STRING(access) (access == ACCESS_DATA_LOAD ? "LOAD" : (access == ACCESS_DATA_STORE ? "STORE" : "INSN_FETCH"))

/* Return value from get_phys_addr* */
#define TRANSLATE_FAIL    1
#define TRANSLATE_SUCCESS 0

#define HOST_PAGE_ALIGN(addr) (((addr) + tlib_host_page_size - 1) & tlib_host_page_mask)

/* same as PROT_xxx */
#define PAGE_READ  (1 << ACCESS_DATA_LOAD)
#define PAGE_WRITE (1 << ACCESS_DATA_STORE)
#define PAGE_EXEC  (1 << ACCESS_INST_FETCH)
#define PAGE_BITS  (PAGE_READ | PAGE_WRITE | PAGE_EXEC)
#define PAGE_VALID 0x0008
/* original state of the write flag (used when tracking self-modifying
   code */
#define PAGE_WRITE_ORG 0x0010

#define CPU_DUMP_CODE 0x00010000

/* Denotes that the TCGv cpu_pc chould be used in stack change announcement */
#define PROFILER_TCG_PC ((target_ulong) - 1lu)

enum GUEST_PROFILER_ANNOUNCEMENT {
    STACK_FRAME_POP = 0,
    STACK_FRAME_ADD = 1,
    STACK_FRAME_NO_CHANGE = 2,
};

void cpu_abort(CPUState *env, const char *fmt, ...);
extern CPUState *cpu;

static inline uint32_t is_page_access_valid(uint32_t page_protection_bits, uint32_t access_type)
{
    uint32_t access_type_mask;

    switch(access_type) {
        case ACCESS_DATA_LOAD:
            access_type_mask = PAGE_READ;
            break;
        case ACCESS_DATA_STORE:
            access_type_mask = PAGE_WRITE;
            break;
        case ACCESS_INST_FETCH:
            access_type_mask = PAGE_EXEC;
            break;
        default:
            tlib_abortf("Incorrect access type %d", access_type);
            __builtin_unreachable();
    }

    return !!(page_protection_bits & access_type_mask);
}

/* Flags for use in ENV->INTERRUPT_PENDING.

   The numbers assigned here are non-sequential in order to preserve
   binary compatibility with the vmstate dump.  Bit 0 (0x0001) was
   previously used for CPU_INTERRUPT_EXIT, and is cleared when loading
   the vmstate dump.  */

/* External hardware interrupt pending.  This is typically used for
   interrupts from devices.  */
#define CPU_INTERRUPT_HARD 0x0002

/* Exit the current TB.  This is typically used when some system-level device
   makes some change to the memory mapping.  E.g. the a20 line change.  */
#define CPU_INTERRUPT_EXITTB 0x0004

/* Debug event pending.  */
#define CPU_INTERRUPT_DEBUG 0x0080

/* Several target-specific external hardware interrupts.  Each target/cpu.h
   should define proper names based on these defines.  */
#define CPU_INTERRUPT_TGT_EXT_0 0x0008
#define CPU_INTERRUPT_TGT_EXT_1 0x0010
#define CPU_INTERRUPT_TGT_EXT_2 0x0040
#define CPU_INTERRUPT_TGT_EXT_3 0x0200
#define CPU_INTERRUPT_TGT_EXT_4 0x1000

/* Several target-specific internal interrupts.  These differ from the
   preceeding target-specific interrupts in that they are intended to
   originate from within the cpu itself, typically in response to some
   instruction being executed.  These, therefore, are not masked while
   single-stepping within the debugger.  */
#define CPU_INTERRUPT_TGT_INT_0 0x0100
#define CPU_INTERRUPT_TGT_INT_1 0x0400
#define CPU_INTERRUPT_TGT_INT_2 0x0800

/* First unused bit: 0x2000.  */

typedef void (*CPUInterruptHandler)(CPUState *, int);

extern CPUInterruptHandler cpu_interrupt_handler;

static inline void cpu_interrupt(CPUState *s, int mask)
{
    cpu_interrupt_handler(s, mask);
}

void cpu_reset_interrupt(CPUState *env, int mask);

/* Breakpoint flags */
#define BP_GDB 0x10
#define BP_CPU 0x20

int cpu_breakpoint_insert(CPUState *env, target_ulong pc, int flags, CPUBreakpoint **breakpoint);
int cpu_breakpoint_remove(CPUState *env, target_ulong pc, int flags);
void cpu_breakpoint_remove_by_ref(CPUState *env, CPUBreakpoint *breakpoint);
void cpu_breakpoint_remove_all(CPUState *env, int mask);

int cpu_init(const char *cpu_model);
void cpu_reset(CPUState *s);
int cpu_exec(CPUState *env);

void cpu_resume_after_reset_or_init(CPUState *env);

/* Return the physical page corresponding to a virtual one. Use it
   only for debugging because no protection checks are done. Return -1
   if no page found. */
target_phys_addr_t cpu_get_phys_page_debug(CPUState *env, target_ulong addr);

/* physical memory access */

/* Return the CPU state information bitfield for use in transaction callbacks.
   This may also be called from managed code, and if it itself calls any managed code,
   that code must not throw any exceptions that would propagate through this function. */
uint64_t cpu_get_state_for_memory_transaction(CPUState *env, target_ulong addr, int access_type);

/* MMIO pages are identified by a combination of an IO device index and
   3 flags.  The ROMD code stores the page ram offset in iotlb entry,
   so only a limited number of ids are avaiable.  */

#define IO_MEM_NB_ENTRIES (1 << (TARGET_PAGE_BITS - IO_MEM_SHIFT))

/* Flags stored in the low bits of the TLB virtual address.  These are
   defined so that fast path ram access is all zeros.  */
/* TLB entry should be recalculated before each use - e.g., it contains protected regions that needs to be checked */
#define TLB_ONE_SHOT (1 << 2)
/* Zero if TLB entry is valid.  */
#define TLB_INVALID_MASK (1 << 3)
/* Set if TLB entry references a clean RAM page.  The iotlb entry will
   contain the page physical address.  */
#define TLB_NOTDIRTY (1 << 4)
/* Set if TLB entry is an IO callback.  */
#define TLB_MMIO (1 << 5)

bool is_interrupt_pending(CPUState *env, int mask);
void clear_interrupt_pending(CPUState *env, int mask);
void set_interrupt_pending(CPUState *env, int mask);

#if TARGET_LONG_BITS == 64
uint64_t *get_reg_pointer_64(int reg);
#endif
#if TARGET_LONG_BITS == 32
uint32_t *get_reg_pointer_32(int reg);
#endif

static inline uint64_t instructions_to_cycles(CPUState *env, uint64_t instructions)
{
    double cycles_per_instruction = env->millicycles_per_instruction / 1000.0;
    return (uint64_t)(instructions * cycles_per_instruction);
}

void configure_read_caching(uint64_t address, uint64_t lower_address_count, uint64_t upper_address_count);
void configure_write_caching(uint64_t address, uint64_t lower_address_count, uint64_t upper_address_count);

static inline void cpu_sync_instructions_count(CPUState *cpu)
{
    cpu->instructions_count_value += cpu->instructions_count_declaration;
    cpu->instructions_count_total_value += cpu->instructions_count_declaration;
    cpu->instructions_count_declaration = 0;
}

static inline bool external_mmu_enabled(CPUState *env)
{
    return env->external_mmu_position != EMMU_POS_NONE;
}

static inline ExtMmuRange *external_mmu_find_window_by_id(CPUState *env, uint64_t id)
{
    for(int i = 0; i < env->external_mmu_window_count; ++i) {
        if(env->external_mmu_windows[i].id == id) {
            return &env->external_mmu_windows[i];
        }
    }
    return NULL;
}
