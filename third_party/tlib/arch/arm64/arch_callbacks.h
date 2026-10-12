/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

#include <stdint.h>

uint64_t tlib_read_system_register_interrupt_cpu_interface(uint32_t offset);
void tlib_write_system_register_interrupt_cpu_interface(uint32_t offset, uint64_t value);

uint64_t tlib_read_system_register_generic_timer_64(uint32_t offset);
void tlib_write_system_register_generic_timer_64(uint32_t offset, uint64_t value);

uint32_t tlib_read_system_register_generic_timer_32(uint32_t offset);
void tlib_write_system_register_generic_timer_32(uint32_t offset, uint32_t value);

void tlib_on_execution_mode_changed(uint32_t el, uint32_t is_secure);
void tlib_handle_psci_call(void);

void tlib_on_tcm_mapping_update(int32_t index, uint64_t new_address, uint32_t el01_enabled, uint32_t el2_enabled);

uint64_t tlib_get_random_ulong(void);
