#pragma once

#include <stdint.h>

uint32_t tlib_set_maximum_block_size(uint32_t size);
uint32_t tlib_get_maximum_block_size(void);

void tlib_set_millicycles_per_instruction(uint32_t count);
uint32_t tlib_get_millicycles_per_instruction(void);

void gen_helpers(void);

char *tlib_get_arch();
char *tlib_get_commit();

int32_t tlib_init(char *cpu_name);
int32_t tlib_atomic_memory_state_init(uintptr_t atomic_memory_state_ptr, int32_t atomic_id);
int32_t tlib_store_table_init(uintptr_t store_table_ptr, uint8_t store_table_bits);
void tlib_dispose(void);
uint64_t tlib_get_executed_instructions(void);
void tlib_reset(void);
int32_t tlib_execute(uint32_t max_insns);
void tlib_request_translation_block_interrupt(int);
void tlib_try_interrupt_translation_block(void);
void tlib_set_return_request(void);
void tlib_set_paused(void);
void tlib_clear_paused(void);
int32_t tlib_is_wfi(void);

uint32_t tlib_get_page_size(void);
void tlib_map_range(uint64_t start_addr, uint64_t length);
void tlib_unmap_range(uint64_t start, uint64_t end);
uint32_t tlib_is_range_mapped(uint64_t start, uint64_t end);

void tlib_invalidate_translation_blocks(uintptr_t *regions, uint64_t num_regions);

uint64_t tlib_translate_to_physical_address(uint64_t address, uint32_t access_type);

void tlib_set_irq(int32_t interrupt, int32_t state);
int32_t tlib_is_irq_set(void);

void tlib_add_breakpoint(uint64_t address);
void tlib_remove_breakpoint(uint64_t address);
void tlib_set_block_begin_hook_present(uint32_t val);

uint64_t tlib_get_total_executed_instructions(void);

void tlib_set_translation_cache_configuration(uint64_t min_size, uint64_t max_size);
void tlib_invalidate_translation_cache(void);

void tlib_enable_guest_profiler(int value);

void tlib_set_page_io_accessed(uint64_t address);
void tlib_clear_page_io_accessed(uint64_t address);

int tlib_restore_context(void);
void *tlib_export_state(void);
int32_t tlib_get_state_size(void);

void tlib_set_chaining_enabled(uint32_t val);
uint32_t tlib_get_chaining_enabled(void);

void tlib_set_tb_cache_enabled(uint32_t val);
uint32_t tlib_get_tb_cache_enabled(void);

void tlib_set_block_finished_hook_present(uint32_t val);
void tlib_set_cpu_wfi_state_change_hook_present(uint32_t val);

int32_t tlib_set_return_on_exception(int32_t value);
void tlib_flush_page(uint64_t address);

uint64_t tlib_get_register_value(int reg_number);
void tlib_set_register_value(int reg_number, uint64_t val);

void tlib_set_event_flag(int value);

uint32_t tlib_get_current_tb_disas_flags(void);

uint32_t tlib_get_mmu_windows_count(void);

void tlib_enable_external_window_mmu(uint32_t value);

uint64_t tlib_acquire_mmu_window(uint32_t type);

void tlib_reset_mmu_window(uint64_t id);

void tlib_reset_mmu_windows_covering_address(uint64_t address);

void tlib_reset_all_mmu_windows(void);

void tlib_set_mmu_window_start(uint64_t id, uint64_t addr_start);

void tlib_set_mmu_window_end(uint64_t id, uint64_t addr_end, uint32_t range_end_inclusive);

void tlib_set_window_privileges(uint64_t id, int32_t privileges);

void tlib_set_mmu_window_addend(uint64_t id, uint64_t addend);

uint64_t tlib_get_mmu_window_start(uint64_t id);

uint64_t tlib_get_mmu_window_end(uint64_t id);

int tlib_get_window_privileges(uint64_t id);

uint64_t tlib_get_mmu_window_addend(uint64_t id);

void tlib_enable_pre_opcode_execution_hooks(uint32_t value);
uint32_t tlib_install_pre_opcode_execution_hook(uint64_t mask, uint64_t value);
void tlib_enable_post_opcode_execution_hooks(uint32_t value);
uint32_t tlib_install_post_opcode_execution_hook(uint64_t mask, uint64_t value);

//  Defined in 'arch/*/cpu_registers.c'.
uint32_t tlib_get_register_value_32(int reg_number);
void tlib_set_register_value_32(int reg_number, uint32_t value);

#if TARGET_LONG_BITS == 64
uint64_t tlib_get_register_value_64(int reg_number);
void tlib_set_register_value_64(int reg_number, uint64_t value);
#endif

void tlib_before_save(void *env);

void tlib_after_load(void *env);

void tlib_enable_read_cache(uint64_t access_address, uint64_t not_cached_count, uint64_t cached_count);

void tlib_enable_write_cache(uint64_t access_address, uint64_t not_cached_count, uint64_t cached_count);

uint64_t tlib_get_cpu_state_for_memory_transaction(CPUState *env, uint64_t addr, int access_type);
