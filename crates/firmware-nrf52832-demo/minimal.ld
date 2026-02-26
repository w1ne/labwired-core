MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 512K
  RAM : ORIGIN = 0x20000000, LENGTH = 64K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x00000000 :
  {
    LONG(0x20010000); /* Initial SP: top of 64KB SRAM */
    LONG(Reset + 1);  /* Reset handler (Thumb) */
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
