/* LabWired - Firmware Simulation Platform
 * Copyright (C) 2026 Andrii Shylenko
 *
 * This software is released under the MIT License.
 * See the LICENSE file in the project root for full license information.
 */

MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 128K
  RAM : ORIGIN = 0x20000000, LENGTH = 16K
}

SECTIONS
{
  .vector_table 0x00000000 :
  {
    LONG(0x20004000); /* Initial SP */
    LONG(Reset + 1);  /* Reset Handler (Thumb) */
  } > FLASH

  .text :
  {
    *(.text*)
  } > FLASH

  /DISCARD/ :
  {
    *(.ARM.exidx*)
    *(.note.gnu.build-id*)
  }
}
