/****************************************************************************
 * curiosity-pic32mz-ef/src/pic32mz_boot.c
 *
 * SPDX-License-Identifier: Apache-2.0
 *
 * Derived from the Apache NuttX 12.11.0 flipnclick-pic32mz board
 * (boards/mips/pic32mz/flipnclick-pic32mz), retargeted to the Microchip
 * Curiosity PIC32MZ EF 2.0 (DM320209, PIC32MZ2048EFM144).
 *
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.  The
 * ASF licenses this file to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance with the
 * License.  You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
 * WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.  See the
 * License for the specific language governing permissions and limitations
 * under the License.
 *
 ****************************************************************************/

#include <nuttx/config.h>

#include <nuttx/arch.h>
#include <nuttx/board.h>
#include <nuttx/leds/userled.h>

#include "curiosity-pic32mz-ef.h"

/****************************************************************************
 * Name: pic32mz_boardinitialize
 *
 * Description:
 *   Called early in the initialization, after memory is configured and
 *   before any device is initialized.
 *
 ****************************************************************************/

void pic32mz_boardinitialize(void)
{
  /* LEDs are configured by userled_lower_initialize() in bringup. */
}

#ifdef CONFIG_BOARD_LATE_INITIALIZE
void board_late_initialize(void)
{
  pic32mz_bringup();
}
#endif
