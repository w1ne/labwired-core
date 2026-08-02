/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * Sizes match configs/chips/stm32h735.yaml (the simulator's wiring):
 * FLASH 1 MiB @ 0x08000000, RAM = DTCM 128 KiB @ 0x20000000.
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM : ORIGIN = 0x20000000, LENGTH = 128K
}
