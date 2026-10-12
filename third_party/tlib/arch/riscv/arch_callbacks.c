/*
 *  RISC-V callbacks
 *
 *  Copyright (c) Antmicro
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
#include "callbacks.h"
#include "arch_callbacks.h"

DEFAULT_INT_HANDLER1(uint64_t tlib_get_cpu_time, void)
DEFAULT_INT_HANDLER2(int32_t tlib_handle_custom_instruction, uint64_t id, uint64_t opcode)
DEFAULT_VOID_HANDLER1(void tlib_mip_changed, uint64_t value)
DEFAULT_INT_HANDLER1(uint64_t tlib_read_csr, uint64_t csr)
DEFAULT_VOID_HANDLER2(void tlib_write_csr, uint64_t csr, uint64_t value)
DEFAULT_VOID_HANDLER2(void tlib_handle_post_gpr_access_hook, uint32_t register_index, uint32_t is_write)
DEFAULT_VOID_HANDLER3(void tlib_handle_pre_stack_access_hook, uint64_t address, uint32_t width, uint32_t is_write)
DEFAULT_VOID_HANDLER1(void tlib_clic_clear_edge_interrupt, void)
DEFAULT_VOID_HANDLER1(void tlib_clic_acknowledge_interrupt, void)

DEFAULT_VOID_HANDLER2(void tlib_extpmp_cfg_csr_write, uint32_t register_index, uint64_t value)
DEFAULT_INT_HANDLER1(uint64_t tlib_extpmp_cfg_csr_read, uint32_t register_index)
DEFAULT_VOID_HANDLER2(void tlib_extpmp_address_csr_write, uint32_t register_index, uint64_t value)
DEFAULT_INT_HANDLER1(uint64_t tlib_extpmp_address_csr_read, uint32_t register_index)
DEFAULT_INT_HANDLER1(int32_t tlib_extpmp_is_any_region_locked, void)
DEFAULT_INT_HANDLER3(int32_t tlib_extpmp_find_overlapping, uint64_t addr, uint64_t size, int32_t starting_index)
DEFAULT_INT_HANDLER3(int32_t tlib_extpmp_get_access, uint64_t addr, uint64_t size, int32_t access_type)
