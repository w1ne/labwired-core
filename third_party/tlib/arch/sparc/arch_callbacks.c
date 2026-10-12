/*
 *  SPARC callbacks.
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

DEFAULT_INT_HANDLER1(int32_t tlib_find_best_interrupt, void)

DEFAULT_VOID_HANDLER1(void tlib_acknowledge_interrupt, int32_t number)

DEFAULT_VOID_HANDLER1(void tlib_on_cpu_halted, void)

DEFAULT_VOID_HANDLER1(void tlib_on_cpu_power_down, void)
