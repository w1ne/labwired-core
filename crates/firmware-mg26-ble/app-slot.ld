/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * App-slot image: linked after the 32 KiB UART XMODEM bootloader.
 */

MEMORY
{
  FLASH : ORIGIN = 0x08008000, LENGTH = 3168K
  RAM : ORIGIN = 0x20000000, LENGTH = 512K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x08008000 :
  {
    LONG(0x20080000);
    LONG(Reset | 1);
  } > FLASH

  .text :
  {
    *(.text*)
    *(.rodata*)
  } > FLASH

  /DISCARD/ :
  {
    *(.ARM.exidx*)
    *(.note.gnu.build-id*)
  }
}
