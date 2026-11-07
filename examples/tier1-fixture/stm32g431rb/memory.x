/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * Sizes match configs/chips/stm32g431rb.yaml (the simulator's wiring):
 * STM32G431xB — 128 KB Flash, 22 KB main SRAM (DS12589 Features).
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM : ORIGIN = 0x20000000, LENGTH = 22K
}
