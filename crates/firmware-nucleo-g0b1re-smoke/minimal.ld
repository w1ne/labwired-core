/* LabWired NUCLEO-G0B1RE smoke linker script. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 512K
  RAM : ORIGIN = 0x20000000, LENGTH = 128K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x08000000 :
  {
    LONG(0x20020000); /* Initial SP for the exposed 128 KiB SRAM bank. */
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
