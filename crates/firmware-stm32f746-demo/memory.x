/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * STM32F746 Discovery smoke: flash @ 0x08000000, DTCM RAM @ 0x20000000.
 * Soft-float target: thumbv7em-none-eabi (not eabihf).
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 1M
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}
