/* Minimal linker script for the L476 demo. Only emits the bare vector
 * table (initial SP + reset handler) and .text/.rodata. The simulator's
 * ELF loader picks up the `Reset` symbol as the entry point and the
 * vector table at flash base. */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 96K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x08000000 :
  {
    LONG(0x20018000); /* Initial SP at top of 96 KB SRAM1. */
    LONG(Reset + 1);  /* Reset handler (Thumb bit set). */
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
