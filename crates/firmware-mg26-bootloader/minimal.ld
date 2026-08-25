/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 *
 * The bootloader owns the first 32 KiB of flash; applications link at
 * APP_BASE (0x08008000). No .data/.bss sections: all state lives on the
 * stack, so no runtime init is needed and the image stays position-simple.
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 32K
  RAM : ORIGIN = 0x20000000, LENGTH = 512K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x08000000 :
  {
    LONG(0x20080000); /* Initial SP for 512KB SRAM */
    LONG(Reset | 1);  /* Reset Handler (Thumb bit MUST be set) */
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
