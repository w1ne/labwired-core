/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * SAMD51J19A (Adafruit Metro M4 Express): 512 KB flash @ 0x00000000,
 * 192 KB SRAM @ 0x20000000 (DS60001507).
 */

MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 512K
  RAM   : ORIGIN = 0x20000000, LENGTH = 192K
}
