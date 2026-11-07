ENTRY(_start)

MEMORY
{
  FLASH (rx) : ORIGIN = 0x40080000, LENGTH = 4M
  RAM (rwx)  : ORIGIN = 0x3FFB0000, LENGTH = 320K
}

SECTIONS
{
  .text : ALIGN(4)
  {
    KEEP(*(.entry))
    *(.literal .literal.*)
    *(.text .text.*)
    *(.rodata .rodata.*)
  } > FLASH

  .data : ALIGN(4)
  {
    *(.data .data.*)
  } > RAM

  .bss (NOLOAD) : ALIGN(4)
  {
    *(.bss .bss.*)
    *(COMMON)
  } > RAM
}
