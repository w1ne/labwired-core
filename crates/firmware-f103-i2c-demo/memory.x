/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 * SPDX-License-Identifier: MIT
 *
 * STM32F103RB: 128 KB flash, 20 KB SRAM.
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM   : ORIGIN = 0x20000000, LENGTH = 20K
}
