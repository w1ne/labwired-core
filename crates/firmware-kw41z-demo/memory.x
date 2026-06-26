/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * NXP KW41Z (MKW41Z512VHT4): 512KB flash @0x0, 128KB SRAM. The Kinetis SRAM
 * is split SRAM_L (below 0x2000_0000) / SRAM_U (above); the CPU sees one
 * contiguous 0x1FFF_8000..0x2001_8000 window, with the stack at the top.
 */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 512K
  RAM   : ORIGIN = 0x1FFF8000, LENGTH = 128K
}
