/* Minimal linker script for the F407 demo. Emits a 2-entry vector table
 * (initial SP + reset vector) at flash base and .text/.rodata. The
 * simulator's ELF loader picks up the `Reset` symbol as entry point.
 * No bss/data — the smoke firmware uses only stack + const data. */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 128K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x08000000 :
  {
    LONG(0x20020000); /* Initial SP at top of 128 KB SRAM. */
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
