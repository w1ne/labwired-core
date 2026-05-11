/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 * SPDX-License-Identifier: MIT
 *
 * STM32F407VGT6 — 1 MB flash, 128 KB main SRAM
 * (CCM SRAM at 0x10000000 not declared here)
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 1M
  RAM   : ORIGIN = 0x20000000, LENGTH = 128K
}
