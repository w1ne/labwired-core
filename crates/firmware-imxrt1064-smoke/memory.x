/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * Teensy 4.1 / i.MX RT106x smoke: XIP skipped — link into DTCM @ 0x20000000.
 * Lower 64K = FLASH (vectors + .text), upper 64K = RAM (.data/.bss/stack).
 * Soft-float target: thumbv7em-none-eabi (not eabihf).
 */

MEMORY
{
  FLASH : ORIGIN = 0x20000000, LENGTH = 64K
  RAM   : ORIGIN = 0x20010000, LENGTH = 64K
}
