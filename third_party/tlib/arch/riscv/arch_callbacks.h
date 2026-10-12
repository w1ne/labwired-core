#pragma once

#include <stdint.h>
#include <stdbool.h>

uint64_t tlib_get_cpu_time();

uint64_t tlib_read_csr(uint64_t csr);
void tlib_write_csr(uint64_t csr, uint64_t value);
void tlib_mip_changed(uint64_t value);

int32_t tlib_handle_custom_instruction(uint64_t id, uint64_t opcode);
void tlib_handle_post_gpr_access_hook(uint32_t register_index, uint32_t is_write);
void tlib_handle_pre_stack_access_hook(uint64_t address, uint32_t width, uint32_t is_write);
void tlib_clic_clear_edge_interrupt(void);
void tlib_clic_acknowledge_interrupt(void);

void tlib_extpmp_cfg_csr_write(uint32_t register_index, uint64_t value);
uint64_t tlib_extpmp_cfg_csr_read(uint32_t register_index);
void tlib_extpmp_address_csr_write(uint32_t register_index, uint64_t value);
uint64_t tlib_extpmp_address_csr_read(uint32_t register_index);
int32_t tlib_extpmp_get_access(uint64_t addr, uint64_t size, int32_t access_type);
int32_t tlib_extpmp_find_overlapping(uint64_t addr, uint64_t size, int32_t starting_index);
int32_t tlib_extpmp_is_any_region_locked(void);
