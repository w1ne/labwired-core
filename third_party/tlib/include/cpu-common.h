#pragma once

/* CPU interfaces that are target indpendent.  */

//  TODO: Fix or remove. Using 'WORDS_ALIGNED' is currently broken
//        but it turns out it isn't really required on ARMv7.
//  #if defined(__arm__)
//  #define WORDS_ALIGNED
//  #endif

#include <stdbool.h>

#include "targphys.h"
#include "bswap.h"
#include "tlib-queue.h"

#ifndef CPU_PC
#define CPU_PC(x) x->pc
#endif

/* address in the RAM (different from a physical address) */
typedef uintptr_t ram_addr_t;

/* memory API */

typedef void CPUWriteMemoryFunc(void *opaque, target_phys_addr_t addr, uint32_t value);
typedef uint32_t CPUReadMemoryFunc(void *opaque, target_phys_addr_t addr);

void cpu_register_physical_memory_log(target_phys_addr_t start_addr, ram_addr_t size, ram_addr_t phys_offset,
                                      ram_addr_t region_offset, bool log_dirty);

static inline void cpu_register_physical_memory_offset(target_phys_addr_t start_addr, ram_addr_t size, ram_addr_t phys_offset,
                                                       ram_addr_t region_offset)
{
    cpu_register_physical_memory_log(start_addr, size, phys_offset, region_offset, false);
}

static inline void cpu_register_physical_memory(target_phys_addr_t start_addr, ram_addr_t size, ram_addr_t phys_offset)
{
    cpu_register_physical_memory_offset(start_addr, size, phys_offset, 0);
}

ram_addr_t cpu_get_physical_page_desc(target_phys_addr_t addr);
/* This should only be used for ram local to a device.  */
void *get_ram_ptr(ram_addr_t addr);

void cpu_physical_memory_rw(target_phys_addr_t addr, uint8_t *buf, int len, int is_write);
static inline void cpu_physical_memory_read(target_phys_addr_t addr, void *buf, int len)
{
    cpu_physical_memory_rw(addr, buf, len, 0);
}
static inline void cpu_physical_memory_write(target_phys_addr_t addr, const void *buf, int len)
{
    cpu_physical_memory_rw(addr, (void *)buf, len, 1);
}

uint32_t ldub_phys(target_phys_addr_t addr);
void stb_phys(target_phys_addr_t addr, uint32_t val);

uint32_t lduw_phys(target_phys_addr_t addr);
uint32_t ldl_phys(target_phys_addr_t addr);
uint64_t ldq_phys(target_phys_addr_t addr);
target_ulong ldp_phys(target_phys_addr_t addr);
void stl_phys_notdirty(target_phys_addr_t addr, uint32_t val);
void stq_phys_notdirty(target_phys_addr_t addr, uint64_t val);
void stw_phys(target_phys_addr_t addr, uint32_t val);
void stl_phys(target_phys_addr_t addr, uint32_t val);
void stq_phys(target_phys_addr_t addr, uint64_t val);

void cpu_physical_memory_write_rom(target_phys_addr_t addr, const uint8_t *buf, int len);

#define IO_MEM_SHIFT 3

#define IO_MEM_RAM        (0 << IO_MEM_SHIFT) /* hardcoded offset */
#define IO_MEM_ROM        (1 << IO_MEM_SHIFT) /* hardcoded offset */
#define IO_MEM_UNASSIGNED (2 << IO_MEM_SHIFT)
#define IO_MEM_NOTDIRTY   (3 << IO_MEM_SHIFT)
//  TLB_MMIO (4 << IO_MEM_SHIFT)
#define IO_MEM_EXECUTABLE_IO (8 << IO_MEM_SHIFT)
/* Acts like a ROM when read and like a device when written.  */
#define IO_MEM_ROMD    (1)
#define IO_MEM_SUBPAGE (2)

typedef struct __attribute__((__packed__)) {
    bool dirty : 1;
    bool executable_io_mem : 1;
    bool external_permissions : 1;
} PhysPageDescFlags;

typedef struct PhysPageDesc {
    /* offset in host memory of the page + io_index in the low bits */
    ram_addr_t phys_offset;
    ram_addr_t region_offset;
    PhysPageDescFlags flags;
} PhysPageDesc;

target_ulong virt_to_phys(target_ulong virtual, uint32_t access_type, uint32_t nofault);

void tlib_arch_dispose(void);
void translate_init(void);
