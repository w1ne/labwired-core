/* LabWired - HIL Displacement Showcase Linker Script */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 2M
  RAM : ORIGIN = 0x20000000, LENGTH = 640K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x08000000 :
  {
    LONG(0x200A0000); /* Initial SP (end of RAM) */
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
