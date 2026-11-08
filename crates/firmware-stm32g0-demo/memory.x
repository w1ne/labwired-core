/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * Conservative STM32G0 link bounds: 128 KB flash and 36 KB SRAM, matching
 * the G070RB and G071RB variants. This image also fits the NUCLEO-G0B1RE's
 * larger 512 KB flash / 128 KB SRAM profile.
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM : ORIGIN = 0x20000000, LENGTH = 36K
}
