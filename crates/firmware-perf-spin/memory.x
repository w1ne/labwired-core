/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * Sized for the SMALLEST STM32 in the perf matrix, not for any one board: the
 * reset stack pointer is placed at the end of RAM, so claiming more RAM than a
 * board models would fault before main() on that board.
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 32K
  RAM : ORIGIN = 0x20000000, LENGTH = 8K
}
