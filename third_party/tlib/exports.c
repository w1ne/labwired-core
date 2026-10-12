/*
 *  Common interface for translation libraries.
 *
 *  Copyright (c) Antmicro
 *  Copyright (c) Realtime Embedded
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
#include <stdint.h>
#include "cpu.h"
#include "cpu-all.h"
#include "tcg.h"
#include "tcg-additional.h"
#include "exec-all.h"
#include "tb-helper.h"
#include "unwind.h"

#include "exports.h"

__thread struct unwind_state unwind_state;

static tcg_t stcg;

void gen_helpers(void)
{
#define GEN_HELPER 2
#include "helper.h"
}

static void init_tcg()
{
    stcg.ldb = __ldb_mmu;
    stcg.ldw = __ldw_mmu;
    stcg.ldl = __ldl_mmu;
    stcg.ldq = __ldq_mmu;
    stcg.stb = __stb_mmu;
    stcg.stw = __stw_mmu;
    stcg.stl = __stl_mmu;
    stcg.stq = __stq_mmu;
    tcg_attach(&stcg);
    set_temp_buf_offset(offsetof(CPUState, temp_buf));
    int i;
    for(i = 0; i < NB_MMU_MODES + 1; i++) {
        set_tlb_table_n_0_rwa(i, offsetof(CPUState, tlb_table[i][0].addr_read), offsetof(CPUState, tlb_table[i][0].addr_write),
                              offsetof(CPUState, tlb_table[i][0].addend));
        set_tlb_table_n_0(i, offsetof(CPUState, tlb_table[i][0]));
    }
    set_tlb_entry_addr_rwu(offsetof(CPUTLBEntry, addr_read), offsetof(CPUTLBEntry, addr_write), offsetof(CPUTLBEntry, addend));
    set_sizeof_CPUTLBEntry(sizeof(CPUTLBEntry));
    set_TARGET_PAGE_BITS(TARGET_PAGE_BITS);
    attach_malloc(tlib_malloc);
    attach_realloc(tlib_realloc);
    attach_free(tlib_free);
    tcg_perf_init_labeling();
}

//  This function is unsafe if called with a C# frame above tlib_execute on the stack.
void tlib_try_interrupt_translation_block(void)
{
    if(likely(cpu) && unlikely(cpu->tb_interrupt_request_from_callback)) {
        int request_type = cpu->tb_interrupt_request_from_callback;
        cpu->tb_interrupt_request_from_callback = TB_INTERRUPT_NONE;

        switch(request_type) {
            case TB_INTERRUPT_INCLUDE_LAST_INSTRUCTION:
                //  If last instruction is to be included then we only store the exception and it will
                //  actually be triggered at the end of the currently executing instruction
                cpu->exception_index = MMU_EXTERNAL_FAULT;
                break;
            case TB_INTERRUPT_EXCLUDE_LAST_INSTRUCTION:
                interrupt_current_translation_block(cpu, EXCP_WATCHPOINT);
                break;
            case TB_INTERRUPT_SYNCHRONOUS_EXCEPTION:
                interrupt_current_translation_block_from_current_instruction(cpu, cpu->tb_interrupt_exception_from_callback);
                break;
            default:
                tlib_abort("Unhandled translation block interrupt condition. Aborting!");
                break;
        }
    }

    //  Also do the C side of the exception unwinding process if it has beem requested
    if(unlikely(unwind_state.need_jump)) {
        unwind_state.need_jump = false;
        longjmp(unwind_state.envs[unwind_state.env_idx], 1);
    }
}

//  tlib_get_arch_string return an arch string that is
//  *on purpose* generated compile time so that e.g.
//  strings libtlib.so | grep tlib\,arch=[a-z0-9-]*\,host=[a-z0-9-]*
//  can return the string.
char *tlib_get_arch_string()
{
    return "tlib,arch="
#if defined(TARGET_ARM) || defined(TARGET_ARM64)
           "arm"
#elif defined(TARGET_RISCV)
           "riscv"
#elif defined(TARGET_PPC)
           "ppc"
#elif defined(TARGET_XTENSA)
           "xtensa"
#elif defined(TARGET_I386)
           "i386"
#else
           "unknown"
#endif
           "-"
#if TARGET_LONG_BITS == 32
           "32"
#elif TARGET_LONG_BITS == 64
           "64"
#else
           "unknown"
#endif
           "-"
#ifdef TARGET_WORDS_BIGENDIAN
           "big"
#else
           "little"
#endif
           ",host="
#ifdef HOST_I386
           "i386"
#elif HOST_ARM
           "arm"
#else
           "unknown"
#endif
           "-"
#if HOST_LONG_BITS == 32
           "32"
#elif HOST_LONG_BITS == 64
           "64"
#else
           "unknown"
#endif
        ;
}

char *tlib_get_arch()
{
#if defined(TARGET_RISCV32)
    return "rv32";
#elif defined(TARGET_RISCV64)
    return "rv64";
#elif defined(TARGET_ARM)
    return "arm";
#elif defined(TARGET_ARM64)
    return "arm64";
#elif defined(TARGET_I386)
    return "i386";
#elif defined(TARGET_PPC32)
    return "ppc";
#elif defined(TARGET_PPC64)
    return "ppc64";
#elif defined(TARGET_XTENSA)
    return "xtensa";
#else
    return "unknown";
#endif
}

EXC_POINTER_0(char *, tlib_get_arch)

uint32_t maximum_block_size;

uint32_t tlib_set_maximum_block_size(uint32_t size)
{
    if(size > TCG_MAX_INSNS) {
        tlib_printf(LOG_LEVEL_WARNING, "Limiting maximum block size to %d (%" PRIu32 " requested)\n", TCG_MAX_INSNS, size);
        size = TCG_MAX_INSNS;
    }

    maximum_block_size = size;
    return maximum_block_size;
}

/* GCC 8.1 from MinGW-w64 complains about the size argument potentially being clobbered
 * by a longjmp, but it will not be used after the longjmp in question. */
#ifndef __llvm__
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wclobbered"
#endif
EXC_INT_1(uint32_t, tlib_set_maximum_block_size, uint32_t, size)
#ifndef __llvm__
#pragma GCC diagnostic pop
#endif

uint32_t tlib_get_maximum_block_size()
{
    return maximum_block_size;
}

EXC_INT_0(uint32_t, tlib_get_maximum_block_size)

__attribute__((weak)) void cpu_before_cycles_per_instruction_change(CPUState *env)
{
    //  Empty function for architectures which don't have the function implemented.
}

__attribute__((weak)) void cpu_after_cycles_per_instruction_change(CPUState *env)
{
    //  Empty function for architectures which don't have the function implemented.
}

void tlib_set_millicycles_per_instruction(uint32_t count)
{
    if(env->millicycles_per_instruction == count) {
        return;
    }

    cpu_before_cycles_per_instruction_change(cpu);
    env->millicycles_per_instruction = count;
    cpu_after_cycles_per_instruction_change(cpu);
}

EXC_VOID_1(tlib_set_millicycles_per_instruction, uint32_t, count)

uint32_t tlib_get_millicycles_per_instruction()
{
    return env->millicycles_per_instruction;
}

EXC_INT_0(uint32_t, tlib_get_millicycles_per_instruction)

int32_t tlib_init(char *cpu_name)
{
    init_tcg();
    env = tlib_mallocz(sizeof(CPUState));
    cpu_exec_init(env);
    cpu_exec_init_all();
    gen_helpers();
    translate_init();
    if(cpu_init(cpu_name) != 0) {
        tlib_free(env);
        return -1;
    }
    tlb_flush(env, 1, true);
    tlib_set_maximum_block_size(TCG_MAX_INSNS);
    env->atomic_memory_state = NULL;
    return 0;
}

EXC_INT_1(int32_t, tlib_init, char *, cpu_name)

//  atomic_id should normally be '-1' - then a next free id will be returned
//  passing an id explicitly only makes sense when restoring state after deserialization
int32_t tlib_atomic_memory_state_init(uintptr_t atomic_memory_state_ptr, int32_t atomic_id)
{
    cpu->atomic_memory_state = (atomic_memory_state_t *)atomic_memory_state_ptr;

    return register_in_atomic_memory_state(cpu->atomic_memory_state, atomic_id);
}

EXC_INT_2(int32_t, tlib_atomic_memory_state_init, uintptr_t, atomic_memory_state_ptr, int32_t, atomic_id)

/* Must be called after tlib_atomic_memory_state_init */
int32_t tlib_store_table_init(uintptr_t store_table_ptr, uint8_t store_table_bits)
{
    tlib_assert(cpu->atomic_id != -1);
    //  The size of a single table entry is 8 bytes,
    //  therefore we subtract 3 to leave 2^3 bytes of space available.
    uint8_t max_bits = (sizeof(uintptr_t) * 8) - 3;
    tlib_assert(store_table_bits > 0 && store_table_bits <= max_bits);

    cpu->store_table_bits = store_table_bits;
    cpu->store_table = (store_table_entry_t *)store_table_ptr;
    calculate_hst_mask(cpu);

    //  Use the same id as the atomic memory state, since hst behaves similarly.
    return cpu->atomic_id;
}

EXC_INT_2(int32_t, tlib_store_table_init, uintptr_t, store_table_ptr, uint64_t, store_table_bits)

void tlib_dispose()
{
    tcg_perf_fini_labeling();
    tlib_arch_dispose();
    code_gen_free();
    free_all_page_descriptors();
    //  `tlib_free` is an EXTERNAL_AS, as such we need to clear `cpu` before calling it
    //  to avoid a use-after-free in its wrapper
    CPUState *cpu_copy = cpu;
    cpu = NULL;
    tlib_free(cpu_copy);
    tcg_dispose();
}

EXC_VOID_0(tlib_dispose)

//  this function returns number of instructions executed since the previous call
//  there is `cpu->instructions_count_total_value` that contains the cumulative value
uint64_t tlib_get_executed_instructions()
{
    uint64_t result = cpu->instructions_count_value;
    cpu->instructions_count_value = 0;
    cpu->instructions_count_limit -= result;
    return result;
}

EXC_INT_0(uint64_t, tlib_get_executed_instructions)

uint64_t tlib_get_total_executed_instructions()
{
    return cpu->instructions_count_total_value;
}

EXC_INT_0(uint64_t, tlib_get_total_executed_instructions)

void tlib_reset()
{
    tb_flush(cpu);
    tlb_flush(cpu, 1, false);
    if(unlikely(cpu->cpu_wfi_state_change_hook_present)) {
        if(cpu->was_not_working) {
            //  CPU left WFI on reset
            //  Clean flag, to prevent an edge case, where the CPU is Reset from WFI hook
            //  resulting in infinite recursion
            cpu->was_not_working = false;
            tlib_on_wfi_state_change(false);
        }
    }
    cpu_reset(cpu);
}

EXC_VOID_0(tlib_reset)

//  Does not actually unwind immediately, but sets a flag that requests unwinding at
//  the end of the next reached `EXTERNAL` wrapper function. This will be the one whose
//  wrapped managed function threw an exception.
void tlib_unwind(void)
{
    unwind_state.need_jump = true;
}

__attribute__((weak)) void cpu_on_leaving_reset_state(CPUState *env)
{
    //  Empty function for architectures which don't have the function implemented.
}

void tlib_on_leaving_reset_state()
{
    cpu_on_leaving_reset_state(cpu);
}

EXC_VOID_0(tlib_on_leaving_reset_state)

int32_t tlib_execute(uint32_t max_insns)
{
    if(cpu->instructions_count_value != 0) {
        tlib_abortf("Tried to execute cpu without reading executed instructions count first.");
    }
    cpu->instructions_count_limit = max_insns;

    uint32_t local_counter = 0;
    int32_t result = EXCP_INTERRUPT;
    while((result == EXCP_INTERRUPT) && (cpu->instructions_count_limit > 0)) {
        result = cpu_exec(cpu);

        cpu_sync_instructions_count(cpu);
        local_counter += cpu->instructions_count_value;
        cpu->instructions_count_limit -= cpu->instructions_count_value;
        cpu->instructions_count_value = 0;

        if(cpu->exit_request) {
            cpu->exit_request = 0;
            break;
        }
    }

    //  we need to reset the instructions count value
    //  as this is might be accessed after calling `tlib_execute`
    //  to read the progress
    cpu->instructions_count_value = local_counter;

    return result;
}

EXC_INT_1(int32_t, tlib_execute, int32_t, max_insns)

int tlib_restore_context(void);

extern void *global_retaddr;

//  This function should only be called from at most one level of C -> C# calls, otherwise
//  when the outermost C# method returns the frames of the inner ones will be longjmped over.
void tlib_request_translation_block_interrupt(int shouldSubstractInstruction)
{
    env->tb_interrupt_request_from_callback = shouldSubstractInstruction ? TB_INTERRUPT_EXCLUDE_LAST_INSTRUCTION
                                                                         : TB_INTERRUPT_INCLUDE_LAST_INSTRUCTION;
}

EXC_VOID_1(tlib_request_translation_block_interrupt, int, shouldSubstractInstruction)

void tlib_set_return_request()
{
    cpu->exit_request = 1;
}

EXC_VOID_0(tlib_set_return_request)

int32_t tlib_is_wfi()
{
    return cpu->wfi;
}

EXC_INT_0(int32_t, tlib_is_wfi)

uint32_t tlib_get_page_size()
{
    return TARGET_PAGE_SIZE;
}

EXC_INT_0(uint32_t, tlib_get_page_size)

void tlib_map_range(uint64_t start_addr, uint64_t length)
{
    ram_addr_t phys_offset = start_addr;
    ram_addr_t size = length;
    cpu_register_physical_memory(start_addr, size, phys_offset | IO_MEM_RAM);
}

EXC_VOID_2(tlib_map_range, uint64_t, start_addr, uint64_t, length)

void tlib_unmap_range(uint64_t start, uint64_t end)
{
    uint64_t new_start;

    while(start <= end) {
        unmap_page(start);
        new_start = start + TARGET_PAGE_SIZE;
        if(new_start < start) {
            return;
        }
        start = new_start;
    }
}

EXC_VOID_2(tlib_unmap_range, uint64_t, start, uint64_t, end)

void tlib_register_access_flags_for_range(uint64_t start_address, uint64_t length, uint32_t is_executable_io_mem)
{
    PhysPageDescFlags flags = {
        .executable_io_mem = is_executable_io_mem ? true : false,
    };

    //  If needed split ranges
    uint64_t addr = start_address;
    while(addr < (start_address + length)) {
        phys_page_alloc(addr >> TARGET_PAGE_BITS, flags);
        addr += TARGET_PAGE_SIZE;
    }
    tlib_printf(LOG_LEVEL_DEBUG, "Registering range flags; start_address: 0x%x, length: 0x%x, flags: 0x%x", start_address, length,
                flags);
}
EXC_VOID_3(tlib_register_access_flags_for_range, uint64_t, startAddress, uint64_t, length, uint32_t, is_executable_io_mem)

uint32_t tlib_enable_external_permission_handler_for_range(uint64_t start_address, uint64_t length, uint32_t external_permissions)
{
    PhysPageDesc *pd;

    uint64_t addr = start_address;
    while(addr < (start_address + length)) {
        pd = phys_page_find((target_phys_addr_t)addr >> TARGET_PAGE_BITS);
        if(pd != NULL) {
            pd->flags.external_permissions = external_permissions;
            addr += TARGET_PAGE_SIZE;
            continue;
        }

        PhysPageDescFlags flags = {
            .external_permissions = external_permissions,
        };
        pd = phys_page_alloc(addr >> TARGET_PAGE_BITS, flags);

        addr += TARGET_PAGE_SIZE;
    }
    return 0;
}
EXC_INT_3(uint32_t, tlib_enable_external_permission_handler_for_range, uint64_t, start_address, uint64_t, length, uint32_t,
          external_permissions)

uint32_t tlib_is_range_mapped(uint64_t start, uint64_t end)
{
    PhysPageDesc *pd;

    while(start < end) {
        pd = phys_page_find((target_phys_addr_t)start >> TARGET_PAGE_BITS);
        if(pd != NULL && pd->phys_offset != IO_MEM_UNASSIGNED) {
            return 1;  //  at least one page of this region is mapped
        }
        start += TARGET_PAGE_SIZE;
    }
    return 0;
}

EXC_INT_2(uint32_t, tlib_is_range_mapped, uint64_t, start, uint64_t, end)

void tlib_invalidate_translation_blocks(uintptr_t *regions, uint64_t num_regions)
{
    for(size_t regionIdx = 0; regionIdx < 2 * num_regions; regionIdx += 2) {
        uint64_t start = regions[regionIdx];
        uint64_t end = regions[regionIdx + 1];
        tb_invalidate_phys_page_range_checked(start, end, 0, 0);
    }
}

EXC_VOID_2(tlib_invalidate_translation_blocks, uintptr_t *, regions, uint64_t, num_regions)

uint64_t tlib_translate_to_physical_address(uint64_t address, uint32_t access_type)
{
    if(address > TARGET_ULONG_MAX) {
        return (uint64_t)-1;
    }
    uint64_t ret = virt_to_phys(address, access_type, 1);
    if(ret == TARGET_ULONG_MAX) {
        ret = (uint64_t)-1;
    }
    return ret;
}

EXC_INT_2(uint64_t, tlib_translate_to_physical_address, uint64_t, address, uint32_t, access_type)

void tlib_set_irq(int32_t interrupt, int32_t state)
{
    if(state) {
        cpu_interrupt(cpu, interrupt);
    } else {
        cpu_reset_interrupt(cpu, interrupt);
    }
}

EXC_VOID_2(tlib_set_irq, int32_t, interrupt, int32_t, state)

int32_t tlib_is_irq_set()
{
    return cpu->interrupt_request;
}

EXC_INT_0(int32_t, tlib_is_irq_set)

void tlib_add_breakpoint(uint64_t address)
{
    cpu_breakpoint_insert(cpu, address, BP_GDB, NULL);
}

EXC_VOID_1(tlib_add_breakpoint, uint64_t, address)

void tlib_remove_breakpoint(uint64_t address)
{
    cpu_breakpoint_remove(cpu, address, BP_GDB);
}

EXC_VOID_1(tlib_remove_breakpoint, uint64_t, address)

uint64_t translation_cache_size_min = MIN_CODE_GEN_BUFFER_SIZE;
uint64_t translation_cache_size_max = MAX_CODE_GEN_BUFFER_SIZE;

void tlib_set_translation_cache_configuration(uint64_t min_size, uint64_t max_size)
{
    if(min_size < MIN_CODE_GEN_BUFFER_SIZE) {
        tlib_printf(LOG_LEVEL_WARNING,
                    "Translation cache size %" PRIu64 " is smaller than minimum allowed %" PRIu64
                    ". It will be clamped to minimum",
                    min_size, MIN_CODE_GEN_BUFFER_SIZE);
        min_size = MIN_CODE_GEN_BUFFER_SIZE;
    }
    translation_cache_size_min = min_size;
    if(max_size > MAX_CODE_GEN_BUFFER_SIZE) {
        tlib_printf(LOG_LEVEL_WARNING,
                    "Translation cache size %" PRIu64 " is larger than maximum allowed %" PRIu64
                    ". It will be clamped to maximum",
                    max_size, MAX_CODE_GEN_BUFFER_SIZE);
        max_size = MAX_CODE_GEN_BUFFER_SIZE;
    }
    translation_cache_size_max = max_size;

    if(translation_cache_size_min > translation_cache_size_max) {
        tlib_abortf("Translation cache minimum size %" PRIu64 " is larger than maximum %" PRIu64, translation_cache_size_min,
                    translation_cache_size_max);
    }
}

EXC_VOID_2(tlib_set_translation_cache_configuration, uint64_t, size, int, min_max)

void tlib_invalidate_translation_cache()
{
    if(cpu) {
        tb_flush(cpu);
    }
}

EXC_VOID_0(tlib_invalidate_translation_cache)

int tlib_restore_context()
{
    uintptr_t pc;
    TranslationBlock *tb;

    pc = (uintptr_t)global_retaddr;
    tb = tb_find_pc(pc);
    if(tb == 0) {
        //  this happens when PC is outside RAM or ROM
        return -1;
    }
    return cpu_restore_state_from_tb(cpu, tb, pc);
}

EXC_INT_0(int, tlib_restore_context)

void *tlib_export_state()
{
    return cpu;
}

EXC_POINTER_0(void *, tlib_export_state)

void *tlib_export_external_mmu_state(void)
{
    return cpu->external_mmu_windows;
}

EXC_POINTER_0(void *, tlib_export_external_mmu_state)

int32_t tlib_get_state_size()
{
    //  Cpu state size is reported as
    //  an offset of `current_tb` field
    //  provided by CPU_COMMON definition.
    //  It is a convention that all
    //  architecture-specific, non-pointer
    //  fields should be located in this
    //  range. As a result this size can
    //  be interpreted as an amount of bytes
    //  to store during serialization.
    return (ssize_t)(&((CPUState *)0)->current_tb);
}

EXC_INT_0(int32_t, tlib_get_state_size)

int32_t tlib_get_external_mmu_state_size(void)
{
    return cpu->external_mmu_window_count * sizeof(ExtMmuRange);
}

EXC_INT_0(int32_t, tlib_get_external_mmu_state_size)

void tlib_set_chaining_enabled(uint32_t val)
{
    cpu->chaining_disabled = !val;
}

EXC_VOID_1(tlib_set_chaining_enabled, uint32_t, val)

uint32_t tlib_get_chaining_enabled()
{
    return !cpu->chaining_disabled;
}

EXC_INT_0(uint32_t, tlib_get_chaining_enabled)

void tlib_set_tb_cache_enabled(uint32_t val)
{
    cpu->tb_cache_disabled = !val;
}

EXC_VOID_1(tlib_set_tb_cache_enabled, uint32_t, val)

uint32_t tlib_get_tb_cache_enabled()
{
    return !cpu->tb_cache_disabled;
}

EXC_INT_0(uint32_t, tlib_get_tb_cache_enabled)

void tlib_set_sync_pc_every_instruction_disabled(uint32_t val)
{
    cpu->sync_pc_every_instruction_disabled = val;
}

EXC_VOID_1(tlib_set_sync_pc_every_instruction_disabled, uint32_t, val)

uint32_t tlib_get_sync_pc_every_instruction_disabled()
{
    return cpu->sync_pc_every_instruction_disabled;
}

EXC_INT_0(uint32_t, tlib_get_sync_pc_every_instruction_disabled)

void tlib_set_block_finished_hook_present(uint32_t val)
{
    cpu->block_finished_hook_present = !!val;
}

EXC_VOID_1(tlib_set_block_finished_hook_present, uint32_t, val)

void tlib_set_block_begin_hook_present(uint32_t val)
{
    cpu->block_begin_hook_present = !!val;
}

EXC_VOID_1(tlib_set_block_begin_hook_present, uint32_t, val)

int32_t tlib_set_return_on_exception(int32_t value)
{
    int32_t previousValue = cpu->return_on_exception;
    cpu->return_on_exception = !!value;
    return previousValue;
}

EXC_INT_1(int32_t, tlib_set_return_on_exception, int32_t, value)

void tlib_flush_page(uint64_t address)
{
    tlb_flush_page(cpu, address, false);
}

EXC_VOID_1(tlib_flush_page, uint64_t, address)

void tlib_flush_tlb(bool from_cpu_thread)
{
    tlb_flush(cpu, /* flush_global: */ 1, /* from_generated_code: */ from_cpu_thread);
}
EXC_VOID_1(tlib_flush_tlb, bool, from_cpu_thread)

#define DEFINE_DEFAULT_REGISTER_ACCESSORS(WIDTH)                 \
    uint64_t tlib_get_register_value(int reg_number)             \
    {                                                            \
        return tlib_get_register_value_##WIDTH(reg_number);      \
    }                                                            \
    void tlib_set_register_value(int reg_number, uint64_t value) \
    {                                                            \
        tlib_set_register_value_##WIDTH(reg_number, value);      \
    }

#if TARGET_LONG_BITS == 32
DEFINE_DEFAULT_REGISTER_ACCESSORS(32)
#elif TARGET_LONG_BITS == 64
DEFINE_DEFAULT_REGISTER_ACCESSORS(64)
#else
#error "Unknown number of bits"
#endif

EXC_INT_1(uint64_t, tlib_get_register_value, int, reg_number)
EXC_VOID_2(tlib_set_register_value, int, reg_number, uint64_t, val)

void tlib_set_interrupt_begin_hook_present(uint32_t val)
{
    cpu->interrupt_begin_callback_enabled = !!val;
}

EXC_VOID_1(tlib_set_interrupt_begin_hook_present, uint32_t, val)

void tlib_set_interrupt_end_hook_present(uint32_t val)
{
    //  Supported in RISC-V, ARM, SPARC architectures only
    cpu->interrupt_end_callback_enabled = !!val;
}

EXC_VOID_1(tlib_set_interrupt_end_hook_present, uint32_t, val)

void tlib_on_memory_access_event_enabled(int32_t value)
{
    cpu->tlib_is_on_memory_access_enabled = !!value;
    //  In order to get all of the memory accesses we need to prevent tcg from using the tlb
    tcg_context_use_tlb(!value);
}

EXC_VOID_1(tlib_on_memory_access_event_enabled, int32_t, value)

void tlib_clean_wfi_proc_state(void)
{
    //  Invalidates "Wait for interrupt" state, and makes the core ready to resume execution
    cpu->exception_index &= ~EXCP_WFI;
    cpu->wfi = 0;
}

EXC_VOID_0(tlib_clean_wfi_proc_state)

void tlib_enable_opcodes_counting(uint32_t value)
{
    cpu->count_opcodes = !!value;
}

EXC_VOID_1(tlib_enable_opcodes_counting, uint32_t, value)

uint32_t tlib_get_opcode_counter(uint32_t opcode_id)
{
    return cpu->opcode_counters[opcode_id - 1].counter;
}

EXC_INT_1(uint32_t, tlib_get_opcode_counter, uint32_t, opcode_id)

void tlib_reset_opcode_counters()
{
    for(int i = 0; i < cpu->opcode_counters_size; i++) {
        cpu->opcode_counters[i].counter = 0;
    }
}

EXC_VOID_0(tlib_reset_opcode_counters)

uint32_t tlib_install_opcode_counter(uint32_t opcode, uint32_t mask)
{
    if(cpu->opcode_counters_size == MAX_OPCODE_COUNTERS) {
        //  value 0 should be interpreted as an error;
        //  code calling `tlib_install_opcode_counter` should
        //  handle this properly (and e.g., log an error message)
        return 0;
    }

    cpu->opcode_counters[cpu->opcode_counters_size].opcode = opcode;
    cpu->opcode_counters[cpu->opcode_counters_size].mask = mask;
    cpu->opcode_counters_size++;

    return cpu->opcode_counters_size;
}

EXC_INT_2(uint32_t, tlib_install_opcode_counter, uint32_t, opcode, uint32_t, mask)

void tlib_enable_pre_opcode_execution_hooks(uint32_t value)
{
    env->are_pre_opcode_execution_hooks_enabled = !!value;
    tb_flush(env);
}

EXC_VOID_1(tlib_enable_pre_opcode_execution_hooks, uint32_t, value)

uint32_t tlib_install_pre_opcode_execution_hook(uint64_t mask, uint64_t value)
{
    if(env->pre_opcode_execution_hooks_count == CPU_HOOKS_MASKS_LIMIT) {
        tlib_printf(
            LOG_LEVEL_WARNING,
            "Cannot install another pre opcode execution hook, the maximum number of %d hooks have already been installed",
            CPU_HOOKS_MASKS_LIMIT);
        return -1u;
    }

    uint8_t mask_index = env->pre_opcode_execution_hooks_count++;
    env->pre_opcode_execution_hook_masks[mask_index] =
        (opcode_hook_mask_t) { .mask = (target_ulong)mask, .value = (target_ulong)value };
    return mask_index;
}

EXC_INT_2(uint32_t, tlib_install_pre_opcode_execution_hook, uint64_t, mask, uint64_t, value)

void tlib_enable_post_opcode_execution_hooks(uint32_t value)
{
    env->are_post_opcode_execution_hooks_enabled = !!value;
    tb_flush(env);
}

EXC_VOID_1(tlib_enable_post_opcode_execution_hooks, uint32_t, value)

uint32_t tlib_install_post_opcode_execution_hook(uint64_t mask, uint64_t value)
{
    if(env->post_opcode_execution_hooks_count == CPU_HOOKS_MASKS_LIMIT) {
        tlib_printf(
            LOG_LEVEL_WARNING,
            "Cannot install another post opcode execution hook, the maximum number of %d hooks have already been installed",
            CPU_HOOKS_MASKS_LIMIT);
        return -1u;
    }

    uint8_t mask_index = env->post_opcode_execution_hooks_count++;
    env->post_opcode_execution_hook_masks[mask_index] =
        (opcode_hook_mask_t) { .mask = (target_ulong)mask, .value = (target_ulong)value };
    return mask_index;
}

EXC_INT_2(uint32_t, tlib_install_post_opcode_execution_hook, uint64_t, mask, uint64_t, value)

void tlib_enable_guest_profiler(int value)
{
    if(cpu->guest_profiler_enabled == value) {
        return;
    }

    //  When the state of the guest profiler is changed we have to
    //  invalidate the cache for two reasons:
    //  When the profiler is enabled: to ensure that no block that don't
    //  signal stack changes will be used (function calls will not be detected)
    //  When the profiler is disabled: to ensure that no blocks that
    //  signal stack changes will be used (it's still possible because `tb_flush`
    //  does not interrupt the currently executed block so events might
    //  be sent to a null object but we handle that on the C# side)
    tlib_invalidate_translation_cache();
    cpu->guest_profiler_enabled = !!value;
}
EXC_VOID_1(tlib_enable_guest_profiler, int32_t, value)

uint32_t tlib_get_current_tb_disas_flags()
{
    if(cpu->current_tb == NULL) {
        return 0xFFFFFFFF;
    }

    return cpu->current_tb->disas_flags;
}

EXC_INT_0(uint32_t, tlib_get_current_tb_disas_flags)

void tlib_set_page_io_accessed(uint64_t address)
{
    if(env->io_access_regions_count == MAX_IO_ACCESS_REGIONS_COUNT) {
        tlib_abortf("Couldn't register an IO accessible page 0x%x", address);
    }

    target_ulong page_address = address & ~(TARGET_PAGE_SIZE - 1);

    int i, j;
    for(i = 0; i < env->io_access_regions_count; i++) {
        if(env->io_access_regions[i] == page_address) {
            //  it's already here, just break
            return;
        }

        //  since regions are sorted ascending, this is the right place to put the new entry
        if(env->io_access_regions[i] > page_address) {
            break;
        }
    }

    for(j = env->io_access_regions_count; j > i; j--) {
        env->io_access_regions[j] = env->io_access_regions[j - 1];
    }

    env->io_access_regions[i] = page_address;
    env->io_access_regions_count++;

    tlb_flush_page(env, address, false);
}

EXC_VOID_1(tlib_set_page_io_accessed, uint64_t, address)

void tlib_clear_page_io_accessed(uint64_t address)
{
    target_ulong page_address = address & ~(TARGET_PAGE_SIZE - 1);

    int i, j;
    for(i = 0; i < env->io_access_regions_count; i++) {
        if(env->io_access_regions[i] == page_address) {
            break;
        }
    }

    if(i == env->io_access_regions_count) {
        //  it was not marked as IO
        return;
    }

    for(j = i; j < env->io_access_regions_count - 1; j++) {
        env->io_access_regions[j] = env->io_access_regions[j + 1];
    }
    env->io_access_regions[j] = 0;

    env->io_access_regions_count--;
    tlb_flush_page(env, address, false);
}

EXC_VOID_1(tlib_clear_page_io_accessed, uint64_t, address)

#define ASSERT_EXTERNAL_MMU_ENABLED                                                                 \
    if(!external_mmu_enabled(cpu)) {                                                                \
        tlib_abort("Setting the external MMU parameters, when it is not enabled. Enable it first"); \
    }

#define ENSURE_WINDOW_BY_ID(id)                                                     \
    ({                                                                              \
        ExtMmuRange *window = external_mmu_find_window_by_id(env, id);              \
        if(!window) {                                                               \
            tlib_abortf("Failed to find external MMU window with ID %" PRIu64, id); \
        }                                                                           \
        window;                                                                     \
    })

#define ASSERT_ALIGNED_TO_PAGE_SIZE(addr)          \
    if(((target_ulong)addr) & (~TARGET_PAGE_MASK)) \
        tlib_abortf("MMU ranges must be aligned to the page size (0x%lx), the address 0x%lx is not.", TARGET_PAGE_SIZE, addr);

#define ASSERT_NO_OVERLAP(value, window_type)                                                                                 \
    for(int window_index = 0; window_index < cpu->external_mmu_window_count; window_index++) {                                \
        ExtMmuRange *current_window = &cpu->external_mmu_windows[window_index];                                               \
        if(value >= current_window->range_start && value < current_window->range_end && current_window->type & window_type) { \
            tlib_printf(LOG_LEVEL_DEBUG,                                                                                      \
                        "The addr 0x%lx is already a part of the MMU window of the same type with index %d. Resulting range " \
                        "will overlap!",                                                                                      \
                        value, window_index);                                                                                 \
            break;                                                                                                            \
        }                                                                                                                     \
    }

uint32_t tlib_get_mmu_windows_count(void)
{
    return cpu->external_mmu_window_count;
}
EXC_INT_0(uint32_t, tlib_get_mmu_windows_count)

void tlib_enable_external_window_mmu(uint32_t value)
{
#if !(defined(TARGET_RISCV) || defined(TARGET_ARM) || defined(TARGET_ARM64))
    tlib_printf(LOG_LEVEL_WARNING, "Enabled the external MMU. Please note that this feature is experimental on this platform");
#endif
    cpu->external_mmu_position = value;
}
EXC_VOID_1(tlib_enable_external_window_mmu, uint32_t, value)

void tlib_reset_mmu_window(uint64_t id)
{
    ExtMmuRange *window = ENSURE_WINDOW_BY_ID(id);
    ptrdiff_t index = window - cpu->external_mmu_windows;
    ptrdiff_t elements_to_move = cpu->external_mmu_window_count - index - 1;
    if(elements_to_move > 0) {
        memmove(&cpu->external_mmu_windows[index], &cpu->external_mmu_windows[index + 1], elements_to_move * sizeof(ExtMmuRange));
    }
    cpu->external_mmu_window_count--;
}
EXC_VOID_1(tlib_reset_mmu_window, uint64_t, id)

void tlib_reset_mmu_windows_covering_address(uint64_t address)
{
    //  NB. assumes that a window starting earlier cannot end later (for example because all windows have equal size)
    ExtMmuRange *mmu_windows = env->external_mmu_windows;
    int last_to_remove = find_last_mmu_window_possibly_covering(address);
    if(last_to_remove < 0) {
        return;
    }
    ExtMmuRange *last = &mmu_windows[last_to_remove];
    if(!(last->range_end_inclusive ? address <= last->range_end : address < last->range_end)) {
        return;
    }

    int first_to_remove;
    for(first_to_remove = last_to_remove - 1; first_to_remove >= 0; --first_to_remove) {
        ExtMmuRange *window = &mmu_windows[first_to_remove];
        if(!(window->range_end_inclusive ? address <= window->range_end : address < window->range_end)) {
            break;
        }
    }
    ++first_to_remove;

    int windows_to_remove = last_to_remove - first_to_remove + 1;
    if(windows_to_remove > 0) {
        size_t bytes_to_move = (env->external_mmu_window_count - (last_to_remove + 1)) * sizeof(ExtMmuRange);
        memmove(&mmu_windows[first_to_remove], &mmu_windows[last_to_remove + 1], bytes_to_move);
    }

    env->external_mmu_window_count -= windows_to_remove;
}

EXC_VOID_1(tlib_reset_mmu_windows_covering_address, uint64_t, address)

void tlib_reset_all_mmu_windows(void)
{
    cpu->external_mmu_window_count = 0;
}

EXC_VOID_0(tlib_reset_all_mmu_windows)

uint64_t tlib_acquire_mmu_window(uint32_t type)
{
    ASSERT_EXTERNAL_MMU_ENABLED

    if(cpu->external_mmu_window_count >= cpu->external_mmu_window_capacity) {
        int new_capacity = cpu->external_mmu_window_capacity == 0 ? DEFAULT_EXTERNAL_MMU_RANGE_COUNT
                                                                  : cpu->external_mmu_window_capacity * 2;
        ExtMmuRange *new_array = realloc(cpu->external_mmu_windows, new_capacity * sizeof(ExtMmuRange));
        if(!new_array) {
            tlib_abort("Failed to allocate memory for external MMU windows");
        }
        memset(&new_array[cpu->external_mmu_window_capacity], 0,
               (new_capacity - cpu->external_mmu_window_capacity) * sizeof(ExtMmuRange));
        cpu->external_mmu_windows = new_array;
        cpu->external_mmu_window_capacity = new_capacity;
    }

    ExtMmuRange *new_window = &cpu->external_mmu_windows[cpu->external_mmu_window_count++];
    *new_window = (ExtMmuRange) {
        .id = cpu->external_mmu_window_next_id++,
        .type = (uint8_t)type,
    };
    cpu->external_mmu_windows_unsorted = true;
    return new_window->id;
}

EXC_INT_1(uint64_t, tlib_acquire_mmu_window, uint32_t, type)

void tlib_set_mmu_window_start(uint64_t id, uint64_t addr_start)
{
    ASSERT_EXTERNAL_MMU_ENABLED
    ExtMmuRange *window = ENSURE_WINDOW_BY_ID(id);
    ASSERT_ALIGNED_TO_PAGE_SIZE(addr_start)
#ifdef DEBUG
    ASSERT_NO_OVERLAP(addr_start, window->type)
#endif
    window->range_start = addr_start;
    env->external_mmu_windows_unsorted = true;
}
EXC_VOID_2(tlib_set_mmu_window_start, uint64_t, id, uint64_t, addr_start)

void tlib_set_mmu_window_end(uint64_t id, uint64_t addr_end, uint32_t range_end_inclusive)
{
    ASSERT_EXTERNAL_MMU_ENABLED
    ExtMmuRange *window = ENSURE_WINDOW_BY_ID(id);
    ASSERT_ALIGNED_TO_PAGE_SIZE(range_end_inclusive ? addr_end + 1 : addr_end)
#ifdef DEBUG
    ASSERT_NO_OVERLAP(addr_end, window->type)
#endif
    /* This is not necessary for the MMU to function properly, but it makes it easier to debug when we are using the same
     convention where possible. Only the window that contains the last page of address space will be inclusive at all times */
    if(addr_end != TARGET_ULONG_MAX && range_end_inclusive) {
        addr_end += 1;
        range_end_inclusive = 0;
    }
    window->range_end = addr_end;
    window->range_end_inclusive = !!range_end_inclusive;
}
EXC_VOID_3(tlib_set_mmu_window_end, uint64_t, id, uint64_t, addr_end, uint32_t, range_end_inclusive)

void tlib_set_window_privileges(uint64_t id, int32_t privileges)
{
    ASSERT_EXTERNAL_MMU_ENABLED
    ENSURE_WINDOW_BY_ID(id)->priv = privileges;
}
EXC_VOID_2(tlib_set_window_privileges, uint64_t, id, int32_t, privileges)

void tlib_set_mmu_window_addend(uint64_t id, uint64_t addend)
{
    ASSERT_EXTERNAL_MMU_ENABLED
    ENSURE_WINDOW_BY_ID(id)->addend = addend;
}
EXC_VOID_2(tlib_set_mmu_window_addend, uint64_t, id, uint64_t, addend)

uint64_t tlib_get_mmu_window_start(uint64_t id)
{
    return ENSURE_WINDOW_BY_ID(id)->range_start;
}
EXC_INT_1(uint64_t, tlib_get_mmu_window_start, uint64_t, id)

uint64_t tlib_get_mmu_window_end(uint64_t id)
{
    return ENSURE_WINDOW_BY_ID(id)->range_end;
}
EXC_INT_1(uint64_t, tlib_get_mmu_window_end, uint64_t, id)

int tlib_get_window_privileges(uint64_t id)
{
    return ENSURE_WINDOW_BY_ID(id)->priv;
}
EXC_INT_1(uint64_t, tlib_get_window_privileges, uint64_t, id)

uint64_t tlib_get_mmu_window_addend(uint64_t id)
{
    return ENSURE_WINDOW_BY_ID(id)->addend;
}
EXC_INT_1(uint64_t, tlib_get_mmu_window_addend, uint64_t, id)

void tlib_raise_exception(uint32_t exception)
{
    //  note: this function does interrupt
    //  the execution of the block, so
    //  externally raised exceptions might
    //  not be handled precisely
    env->exception_index = exception;
    env->exit_request = 1;
}
EXC_VOID_1(tlib_raise_exception, uint32_t, exception)

void tlib_set_broadcast_dirty(int enable)
{
    cpu->tb_broadcast_dirty = enable == 0 ? false : true;
}

EXC_VOID_1(tlib_set_broadcast_dirty, int32_t, enable)

char *tlib_get_commit()
{
#if defined(TLIB_COMMIT)
    return stringify(TLIB_COMMIT);
#else
    return "undefined";
#endif
}
EXC_POINTER_0(char *, tlib_get_commit)

void tlib_set_cpu_wfi_state_change_hook_present(uint32_t val)
{
    cpu->cpu_wfi_state_change_hook_present = !!val;
}
EXC_VOID_1(tlib_set_cpu_wfi_state_change_hook_present, uint32_t, val);

__attribute__((weak)) void cpu_before_save(CPUState *env)
{
    //  Empty function for architectures which don't have the function implemented.
}

__attribute__((weak)) void cpu_after_load(CPUState *env)
{
    //  Empty function for architectures which don't have the function implemented.
}

void tlib_before_save(void *env)
{
    cpu_before_save(env);
}

EXC_VOID_1(tlib_before_save, void *, env)

void tlib_after_load(void *env)
{
    CPUState *s = env;
    s->external_mmu_windows = calloc(s->external_mmu_window_capacity, sizeof(ExtMmuRange));
    cpu_after_load(env);
}

EXC_VOID_1(tlib_after_load, void *, env)

void tlib_enable_read_cache(uint64_t access_address, uint64_t not_cached_count, uint64_t cached_count)
{
    configure_read_caching(access_address, not_cached_count, cached_count);
}

EXC_VOID_3(tlib_enable_read_cache, uint64_t, access_address, uint64_t, not_cached_count, uint64_t, cached_count)

void tlib_enable_write_cache(uint64_t access_address, uint64_t not_cached_count, uint64_t cached_count)
{
    configure_write_caching(access_address, not_cached_count, cached_count);
}

EXC_VOID_3(tlib_enable_write_cache, uint64_t, access_address, uint64_t, not_cached_count, uint64_t, cached_count)

uint64_t tlib_get_cpu_state_for_memory_transaction(CPUState *env, uint64_t addr, int access_type)
{
    return cpu_get_state_for_memory_transaction(env, addr, access_type);
}

//  No exception wrapper for performance reasons, any callbacks called by cpu_get_state_for_memory_transaction
//  must not let exceptions propagate
