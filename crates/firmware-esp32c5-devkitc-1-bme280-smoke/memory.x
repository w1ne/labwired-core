ENTRY(_start)

MEMORY
{
  HP_SRAM (rwx) : ORIGIN = 0x40800000, LENGTH = 384K
}

SECTIONS
{
  .text : ALIGN(4)
  {
    KEEP(*(.text._start))
    *(.text .text.*)
    *(.rodata .rodata.*)
  } > HP_SRAM

  .data : ALIGN(4)
  {
    *(.data .data.*)
  } > HP_SRAM

  .bss (NOLOAD) : ALIGN(4)
  {
    *(.bss .bss.*)
    *(COMMON)
  } > HP_SRAM

  /DISCARD/ :
  {
    *(.eh_frame .eh_frame.*)
    *(.comment)
  }
}
