/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * SAM D21 floor: the x15 parts are 32 KB flash / 4 KB SRAM (DS40001882
 * sec. 2). Linking to the smallest orderable geometry lets one image boot
 * on every SAM D21 E/G/J x15-x18 A-variant.
 */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 32K
  RAM : ORIGIN = 0x20000000, LENGTH = 4K
}
