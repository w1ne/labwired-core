/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 */

MEMORY
{
  /* Standard Cortex-M layout often used in examples */
  FLASH : ORIGIN = 0x00000000, LENGTH = 128K
  RAM : ORIGIN = 0x20000000, LENGTH = 128K
}
