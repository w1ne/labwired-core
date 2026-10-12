/*
 * Copyright (c) Antmicro
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

#include <stdint.h>
#include "arch_exports_common.h"

uint32_t tlib_has_el3();
bool tlib_is_gic_or_generic_timer_system_register(char *name);
void tlib_register_tcm_region(uint32_t address, uint64_t size, uint64_t region_index);
void tlib_set_available_els(bool el2_enabled, bool el3_enabled);
void tlib_set_current_el(uint32_t el);
void tlib_set_mpu_regions_count(uint32_t count);
void tlib_psci_handler_enable(uint32_t mode);
void tlib_set_gic_cpu_register_interface_version(uint32_t iface_version);
