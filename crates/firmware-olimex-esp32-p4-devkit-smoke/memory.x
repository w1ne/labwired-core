ENTRY(_start)

MEMORY
{
  RAM (rwx) : ORIGIN = 0x4FF00000, LENGTH = 768K
}

SECTIONS
{
  .text : ALIGN(4) { KEEP(*(.text._start)) *(.text .text.*) } > RAM
  .rodata : ALIGN(4) { *(.rodata .rodata.*) } > RAM
  .data : ALIGN(4) { *(.data .data.*) } > RAM
  .bss (NOLOAD) : ALIGN(4) { *(.bss .bss.*) *(COMMON) } > RAM
  /DISCARD/ : { *(.eh_frame .eh_frame.*) }
}
