ENTRY(_start)

PHDRS
{
  iram PT_LOAD FLAGS(5);
}

SECTIONS
{
  .text 0x40378000 : ALIGN(4)
  {
    KEEP(*(.entry))
    *(.literal .literal.*)
    *(.text .text.*)
  } :iram

  /DISCARD/ :
  {
    *(.comment)
    *(.note*)
    *(.xtensa.info)
  }
}
