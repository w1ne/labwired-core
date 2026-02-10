MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 1M
  RAM   : ORIGIN = 0x20000000, LENGTH = 1M
}

SECTIONS
{
  .text :
  {
    LONG(ORIGIN(RAM) + LENGTH(RAM));
    LONG(Reset + 1);
    *(.text*)
  } > FLASH

  /DISCARD/ :
  {
    *(.ARM.exidx*)
    *(.note.gnu.build-id*)
  }
}
