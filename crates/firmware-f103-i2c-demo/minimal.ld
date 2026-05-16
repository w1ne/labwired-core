/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 * SPDX-License-Identifier: MIT
 *
 * STM32F103RB: 128 KB flash @ 0x08000000, 20 KB SRAM @ 0x20000000.
 *
 * Reset's symbol value already has bit 0 set by rustc (Thumb function),
 * so the vector-table entry is the literal `Reset` address — no `+1`
 * arithmetic needed (would cancel the Thumb bit and INVSTATE on real
 * silicon).
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM   : ORIGIN = 0x20000000, LENGTH = 20K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x08000000 :
  {
    LONG(0x20005000); /* Initial SP — top of F103RB's 20 KiB SRAM */
    LONG(Reset);      /* Reset handler (Thumb bit already in symbol) */
  } > FLASH

  .text :
  {
    *(.text*)
    *(.rodata*)
  } > FLASH

  /DISCARD/ :
  {
    *(.ARM.exidx*)
    *(.ARM.extab*)
    *(.note.gnu.build-id*)
  }
}
