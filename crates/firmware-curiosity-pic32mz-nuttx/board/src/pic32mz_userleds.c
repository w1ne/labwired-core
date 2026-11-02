/****************************************************************************
 * curiosity-pic32mz-ef/src/pic32mz_userleds.c
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

#include <stdint.h>
#include <stdbool.h>

#include <nuttx/board.h>
#include <arch/board/board.h>

#include "pic32mz_gpio.h"
#include "curiosity-pic32mz-ef.h"

#ifndef CONFIG_ARCH_LEDS

static const pinset_t g_ledcfg[BOARD_NLEDS] =
{
  GPIO_LED_1, GPIO_LED_2, GPIO_LED_3
};

uint32_t board_userled_initialize(void)
{
  int i;

  for (i = 0; i < BOARD_NLEDS; i++)
    {
      pic32mz_configgpio(g_ledcfg[i]);
    }

  return BOARD_NLEDS;
}

void board_userled(int led, bool ledon)
{
  if ((unsigned)led < BOARD_NLEDS)
    {
      pic32mz_gpiowrite(g_ledcfg[led], ledon);
    }
}

void board_userled_all(uint32_t ledset)
{
  int i;

  for (i = 0; i < BOARD_NLEDS; i++)
    {
      pic32mz_gpiowrite(g_ledcfg[i], (ledset & (1 << i)) != 0);
    }
}

#endif /* !CONFIG_ARCH_LEDS */
