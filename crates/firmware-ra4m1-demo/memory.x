/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * R7FA4M1AB (Arduino Uno R4 Minima): 256 KB flash @ 0x00000000,
 * 32 KB SRAM @ 0x20000000 (R01UH0887).
 */

MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 32K
}
