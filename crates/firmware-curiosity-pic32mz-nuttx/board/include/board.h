/****************************************************************************
 * curiosity-pic32mz-ef/include/board.h (derived from NuttX 12.11.0 flipnclick-pic32mz)
 *
 * SPDX-License-Identifier: Apache-2.0
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

#ifndef __BOARDS_MIPS_PIC32MZ_CURIOSITY_PIC32MZ_EF_INCLUDE_BOARD_H
#define __BOARDS_MIPS_PIC32MZ_CURIOSITY_PIC32MZ_EF_INCLUDE_BOARD_H

/****************************************************************************
 * Included Files
 ****************************************************************************/

#include <nuttx/config.h>

#ifndef __ASSEMBLY__
#  include <stdbool.h>
#endif

/****************************************************************************
 * Pre-processor Definitions
 ****************************************************************************/

/* Clocking *****************************************************************/

/* Oscillators (DM320209 user guide DS70005400, bill of materials)
 *
 * - Y1, a 12 MHz DSC6011 MEMS oscillator, drives OSC1 (EC mode).
 * - A 32.768 kHz oscillator drives SOSCI.
 *
 * The system PLL runs from the 8 MHz internal FRC, as the Harmony 3
 * default configuration for this board does: 8 MHz / 1 * 50 / 2 = 200 MHz.
 */

#define BOARD_POSC_FREQ        12000000  /* Primary oscillator (Y1, 12MHz DSC6011) */
#define BOARD_SOSC_FREQ        32768     /* Secondary oscillator, 32.768kHz */

/* Oscillator modes.
 *
 * - BOARD_POSC_ECMODE:  An external oscillator is connected to OSC1/OSC2
 * - BOARD_POSC_HSMODE:  An external crystal or resonator is connected to
 *                       OSC1/OSC2
 */

#define BOARD_FNOSC_SPLL       1         /* Use system PLL */
#define BOARD_POSC_ECMODE      1         /* External clock (EC) mode */
#define BOARD_POSC_SWITCH      1         /* Enable clock switching */
#undef  BOARD_POSC_FSCM                  /* Disable clock monitoring */

/* PLL configuration and resulting CPU clock.
 * CPU_CLOCK = ((POSC_FREQ / IDIV) * MULT) / ODIV
 */

#define BOARD_FPLLICLK_FRC     1         /* System PLL input is the 8MHz FRC */
#define BOARD_PLL_INPUT        8000000
#define BOARD_PLL_IDIV         1         /* PLL input divider */
#define BOARD_PLL_MULT         50        /* PLL multiplier */
#define BOARD_PLL_ODIV         2         /* PLL output divider */

#define BOARD_CPU_CLOCK        200000000 /* CPU clock: 200MHz = (8MHz / 1) * 50 / 2) */

/* Peripheral clocks */

/* PBCLK1
 *   Peripherals: OSC2 pin
 *
 * NOTES:
 *   - PBCLK1 is used by system modules and cannot be turned off
 *   - PBCLK1 divided by 2 is available on the OSC2 pin in certain clock
 *     modes.
 */

#define BOARD_PB1DIV           5         /* Divider = 5 */
#define BOARD_PBCLK1           40000000  /* PBCLK1 frequency = 200MHz/5 = 40MHz */

/* PBCLK2
 *   Peripherals: PMP, I2C, UART, SPI
 */

#define BOARD_PBCLK2_ENABLE    1         /* Enable PBCLK2 */
#define BOARD_PB2DIV           2         /* Divider = 2 */
#define BOARD_PBCLK2           100000000 /* PBCLK2 frequency = 200MHz/2 = 100MHz */

/* PBCLK3
 *   Peripherals: ADC, Comparator, Timers, Output Compare, Input Compare
 *
 * NOTES:
 *   - Timer 1 uses SOSC
 */

#define BOARD_PBCLK3_ENABLE    1         /* Enable PBCLK3 */
#define BOARD_PB3DIV           4         /* Divider = 4 */
#define BOARD_PBCLK3           50000000  /* PBCLK3 frequency = 200MHz/4 = 50MHz */

/* PBCLK4
 *   Peripherals: Ports
 */

#define BOARD_PBCLK4_ENABLE    1         /* Enable PBCLK4 */
#define BOARD_PB4DIV           2         /* Divider = 2 */
#define BOARD_PBCLK4           100000000 /* PBCLK4 frequency = 200MHz/2 = 100MHz */

/* PBCLK5
 *   Peripherals: Flash, Crypto, RND, USB, CAN, Ethernet, SQI
 *
 * NOTES:
 *   - PBCLK5 is used to fetch data from/to the Flash Controller, while the
 *     FRC clock is used for programming
 */

#define BOARD_PBCLK5_ENABLE    1         /* Enable PBCLK5 */
#define BOARD_PB5DIV           2         /* Divider = 2 */
#define BOARD_PBCLK5           100000000 /* PBCLK5 frequency = 200MHz/2 = 100MHz */

/* PBCLK6
 *   Peripherals:
 */

#undef BOARD_PBCLK6_ENABLE

/* PBCLK7
 *   Peripherals:  CPU, Deadman timer
 */

#undef BOARD_PBCLK7_ENABLE

/* PBCLK8
 *   Peripherals: EBI
 */

#undef BOARD_PBCLK8_ENABLE

/* Watchdog pre-scaler (re-visit) */

#define BOARD_WD_PRESCALER     1048576   /* Watchdog pre-scaler */

/* Ethernet MII clocking.
 *
 * The clock divider used to create the MII Management Clock (MDC).  The MIIM
 * module uses the PBCLK5 as an input clock.  According to the IEEE 802.3
 * Specification this should be no faster than 2.5 MHz. However, some PHYs
 * support clock rates up to 12.5 MHz.
 */

#define BOARD_EMAC_MIIM_DIV    40        /* Ideal: 100MHz/40 = 2.5MHz */

/* LED definitions **********************************************************/

/* The Curiosity PIC32MZ EF 2.0 has three user LEDs (LED1 RJ7, LED2 RK7,
 * LED3 RJ3).  They are left to the application (CONFIG_ARCH_LEDS=n).
 */

#define BOARD_LED_1      0
#define BOARD_LED_2      1
#define BOARD_LED_3      2
#define BOARD_NLEDS      3

#define BOARD_LED_1_BIT  (1 << BOARD_LED_1)
#define BOARD_LED_2_BIT  (1 << BOARD_LED_2)
#define BOARD_LED_3_BIT  (1 << BOARD_LED_3)

/* UARTS ********************************************************************/

/* UART2 is wired to the PKoB4 debugger's virtual COM port: U2TX on RG9 and
 * U2RX on RE9 (DM320209 user guide DS70005400, schematic).
 */

#define BOARD_U2RX_PPS  U2RXR_RPE9
#define BOARD_U2TX_PPS  U2TX_RPG9R

/****************************************************************************
 * Public Types
 ****************************************************************************/

#ifndef __ASSEMBLY__

/****************************************************************************
 * Inline Functions
 ****************************************************************************/

#ifdef __cplusplus
#define EXTERN extern "C"
extern "C"
{
#else
#define EXTERN extern
#endif

/****************************************************************************
 * Public Function Prototypes
 ****************************************************************************/

#undef EXTERN
#ifdef __cplusplus
}
#endif

#endif /* __ASSEMBLY__ */
#endif /* __BOARDS_MIPS_PIC32MZ_CURIOSITY_PIC32MZ_EF_INCLUDE_BOARD_H */
