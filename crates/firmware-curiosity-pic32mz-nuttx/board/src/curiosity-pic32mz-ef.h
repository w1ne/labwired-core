/****************************************************************************
 * curiosity-pic32mz-ef/src/curiosity-pic32mz-ef.h
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

#ifndef __CURIOSITY_PIC32MZ_EF_SRC_CURIOSITY_PIC32MZ_EF_H
#define __CURIOSITY_PIC32MZ_EF_SRC_CURIOSITY_PIC32MZ_EF_H

#include <nuttx/config.h>
#include <nuttx/compiler.h>
#include <stdint.h>

#include "pic32mz_gpio.h"

/* User LEDs (DM320209 user guide DS70005400; Harmony 3 bsp
 * pic32mz_ef_curiosity_v2): LED1 RJ7, LED2 RK7, LED3 RJ3, active high.
 */

#define GPIO_LED_1  (GPIO_OUTPUT | GPIO_VALUE_ZERO | GPIO_PORTJ | GPIO_PIN7)
#define GPIO_LED_2  (GPIO_OUTPUT | GPIO_VALUE_ZERO | GPIO_PORTK | GPIO_PIN7)
#define GPIO_LED_3  (GPIO_OUTPUT | GPIO_VALUE_ZERO | GPIO_PORTJ | GPIO_PIN3)

#ifndef __ASSEMBLY__

int pic32mz_bringup(void);

#endif /* __ASSEMBLY__ */
#endif /* __CURIOSITY_PIC32MZ_EF_SRC_CURIOSITY_PIC32MZ_EF_H */
