/* Fallback linker script (prefer cortex-m-rt's -Tlink.x + memory.x). */
MEMORY
{
  FLASH : ORIGIN = 0x10000000, LENGTH = 4096K
  RAM : ORIGIN = 0x20000000, LENGTH = 520K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x10000000 :
  {
    LONG(0x20082000); /* top of 520KB SRAM */
    LONG(Reset + 1);
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
