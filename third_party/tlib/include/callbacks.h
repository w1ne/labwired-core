#pragma once

#include <stdint.h>
#include <stdlib.h>
#include "infrastructure.h"

#define DEFAULT_VOID_HANDLER0(NAME)   \
    NAME(void) __attribute__((weak)); \
                                      \
    NAME(void) { }

#define DEFAULT_VOID_HANDLER1(NAME, PARAM1) \
    NAME(PARAM1) __attribute__((weak));     \
                                            \
    NAME(PARAM1) { }

#define DEFAULT_VOID_HANDLER2(NAME, PARAM1, PARAM2) \
    NAME(PARAM1, PARAM2) __attribute__((weak));     \
                                                    \
    NAME(PARAM1, PARAM2) { }

#define DEFAULT_VOID_HANDLER3(NAME, PARAM1, PARAM2, PARAM3) \
    NAME(PARAM1, PARAM2, PARAM3) __attribute__((weak));     \
                                                            \
    NAME(PARAM1, PARAM2, PARAM3) { }

#define DEFAULT_VOID_HANDLER4(NAME, PARAM1, PARAM2, PARAM3, PARAM4) \
    NAME(PARAM1, PARAM2, PARAM3, PARAM4) __attribute__((weak));     \
                                                                    \
    NAME(PARAM1, PARAM2, PARAM3, PARAM4) { }

#define DEFAULT_VOID_HANDLER5(NAME, PARAM1, PARAM2, PARAM3, PARAM4, PARAM5) \
    NAME(PARAM1, PARAM2, PARAM3, PARAM4, PARAM5) __attribute__((weak));     \
                                                                            \
    NAME(PARAM1, PARAM2, PARAM3, PARAM4, PARAM5) { }

#define DEFAULT_INT_HANDLER1(NAME, PARAM1) \
    NAME(PARAM1) __attribute__((weak));    \
                                           \
    NAME(PARAM1)                           \
    {                                      \
        return 0;                          \
    }

#define DEFAULT_INT_HANDLER2(NAME, PARAM1, PARAM2) \
    NAME(PARAM1, PARAM2) __attribute__((weak));    \
                                                   \
    NAME(PARAM1, PARAM2)                           \
    {                                              \
        return 0;                                  \
    }

#define DEFAULT_INT_HANDLER3(NAME, PARAM1, PARAM2, PARAM3) \
    NAME(PARAM1, PARAM2, PARAM3) __attribute__((weak));    \
                                                           \
    NAME(PARAM1, PARAM2, PARAM3)                           \
    {                                                      \
        return 0;                                          \
    }

#define DEFAULT_INT_HANDLER4(NAME, PARAM1, PARAM2, PARAM3, PARAM4) \
    NAME(PARAM1, PARAM2, PARAM3, PARAM4) __attribute__((weak));    \
                                                                   \
    NAME(PARAM1, PARAM2, PARAM3, PARAM4)                           \
    {                                                              \
        return 0;                                                  \
    }

#define DEFAULT_INT_HANDLER3(NAME, PARAM1, PARAM2, PARAM3) \
    NAME(PARAM1, PARAM2, PARAM3) __attribute__((weak));    \
                                                           \
    NAME(PARAM1, PARAM2, PARAM3)                           \
    {                                                      \
        return 0;                                          \
    }

#define DEFAULT_PTR_HANDLER1(NAME, PARAM1) \
    NAME(PARAM1) __attribute__((weak));    \
                                           \
    NAME(PARAM1)                           \
    {                                      \
        return NULL;                       \
    }

uint64_t tlib_read_byte(uint64_t address, uint64_t cpustate);
uint64_t tlib_read_word(uint64_t address, uint64_t cpustate);
uint64_t tlib_read_double_word(uint64_t address, uint64_t cpustate);
uint64_t tlib_read_quad_word(uint64_t address, uint64_t cpustate);
void tlib_write_byte(uint64_t address, uint64_t value, uint64_t cpustate);
void tlib_write_word(uint64_t address, uint64_t value, uint64_t cpustate);
void tlib_write_double_word(uint64_t address, uint64_t value, uint64_t cpustate);
void tlib_write_quad_word(uint64_t address, uint64_t value, uint64_t cpustate);
void *tlib_guest_offset_to_host_ptr(uint64_t offset);
int32_t tlib_mmu_fault_external_handler(uint64_t addr, int32_t access_type, uint64_t window_id, int32_t first_try);
void tlib_invalidate_tb_in_other_cpus(uintptr_t start, uintptr_t end);
void tlib_update_instruction_counter(int32_t value);
void tlib_handle_pre_opcode_execution_hook(uint32_t id, uint64_t pc, uint64_t opcode);
void tlib_handle_post_opcode_execution_hook(uint32_t id, uint64_t pc, uint64_t opcode);
uint64_t tlib_get_total_elapsed_cycles(void);
uint32_t tlib_get_mp_index(void);

void *tlib_malloc(size_t size);
void *tlib_realloc(void *ptr, size_t size);
void tlib_free(void *ptr);

void tlib_abort(char *message);
void tlib_log(int32_t level, char *message);

void tlib_on_translation_block_find_slow(uint64_t pc);
uint32_t tlib_on_block_begin(uint64_t address, uint32_t size);
void tlib_on_translation_cache_size_change(uint64_t new_size);
void tlib_on_block_translation(uint64_t start, uint32_t size, uint32_t flags);
extern int32_t tlib_is_on_block_translation_enabled;
void tlib_set_on_block_translation_enabled(int32_t value);
void tlib_on_block_finished(uint64_t pc, uint32_t executed_instructions);
void tlib_on_interrupt_begin(uint64_t exception_index);
void tlib_on_interrupt_end(uint64_t exception_index);
void tlib_profiler_announce_stack_change(uint64_t current_address, uint64_t current_return_address,
                                         uint64_t current_instructions_count, int32_t is_frame_add);
void tlib_profiler_announce_context_change(uint64_t context_id);
void tlib_on_memory_access(uint64_t pc, uint32_t operation, uint64_t addr, uint32_t width, uint64_t value);
void tlib_profiler_announce_stack_pointer_change(uint64_t address, uint64_t old_stack_pointer, uint64_t stack_pointer,
                                                 uint64_t current_instructions_count);
void tlib_on_memory_access_event_enabled(int32_t value);
void tlib_mass_broadcast_dirty(void *list_start, int size);
void *tlib_get_dirty_addresses_list(void *size);

uint32_t tlib_is_in_debug_mode(void);

void tlib_clean_wfi_proc_state(void);
void tlib_on_wfi_state_change(int32_t is_in_wfi);

uint32_t tlib_is_memory_disabled(uint64_t start, uint64_t size);
uint32_t tlib_check_external_permissions(uint64_t address);
