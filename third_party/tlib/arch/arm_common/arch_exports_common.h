/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

#include <stdint.h>

uint32_t tlib_check_system_register_access(const char *name, bool is_write);
uint32_t tlib_create_system_registers_array(char ***array);
uint64_t tlib_get_system_register(const char *name);
void tlib_set_system_register(const char *name, uint64_t value);
