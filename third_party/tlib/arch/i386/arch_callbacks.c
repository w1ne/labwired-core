/*
 *  x86 callbacks.
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

DEFAULT_INT_HANDLER1(uint8_t tlib_read_byte_from_port, uint16_t address)

DEFAULT_INT_HANDLER1(uint16_t tlib_read_word_from_port, uint16_t address)

DEFAULT_INT_HANDLER1(uint32_t tlib_read_double_word_from_port, uint16_t address)

DEFAULT_VOID_HANDLER2(void tlib_write_byte_to_port, uint16_t address, uint8_t value)

DEFAULT_VOID_HANDLER2(void tlib_write_word_to_port, uint16_t address, uint16_t value)

DEFAULT_VOID_HANDLER2(void tlib_write_double_word_to_port, uint16_t address, uint32_t value)

DEFAULT_INT_HANDLER1(int32_t tlib_get_pending_interrupt, void)

DEFAULT_INT_HANDLER1(uint64_t tlib_get_instruction_count, void)

DEFAULT_VOID_HANDLER1(void tlib_set_tsc_deadline_value, uint64_t value);

DEFAULT_VOID_HANDLER1(void tlib_set_apic_base_value, uint64_t value);
