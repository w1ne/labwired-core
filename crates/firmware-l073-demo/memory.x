/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * STM32L073RZ: 192 KB flash @ 0x08000000, 20 KB SRAM @ 0x20000000
 * (DS10685). The 6 KB data EEPROM (0x08080000) is not used by the demo.
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 192K
  RAM : ORIGIN = 0x20000000, LENGTH = 20K
}
