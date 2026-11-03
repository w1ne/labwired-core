/* LabWired NUCLEO-G431RB smoke linker script. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM : ORIGIN = 0x20000000, LENGTH = 32K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x08000000 :
  {
    LONG(0x20008000); /* Initial SP at the top of the exposed 32 KiB SRAM. */
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
