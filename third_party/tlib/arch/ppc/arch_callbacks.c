/*
 *  PPC callbacks.
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
#include "callbacks.h"
#include "arch_callbacks.h"

DEFAULT_INT_HANDLER1(uint32_t tlib_read_tbl, void)

DEFAULT_INT_HANDLER1(uint32_t tlib_read_tbu, void)

DEFAULT_INT_HANDLER1(uint64_t tlib_read_decrementer, void)

DEFAULT_VOID_HANDLER1(void tlib_write_decrementer, uint64_t value)

DEFAULT_INT_HANDLER1(uint32_t tlib_is_vle_enabled, void)
