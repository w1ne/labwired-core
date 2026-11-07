/* Focused NUCLEO-L4A6ZG smoke linker script; SRAM1 only. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM : ORIGIN = 0x20000000, LENGTH = 256K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x08000000 :
  {
    LONG(0x20040000); /* Top of the exposed 256 KiB SRAM1 region. */
    LONG(Reset + 1);  /* Reset handler (Thumb). */
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
