ENTRY(_start)

SECTIONS
{
    . = 0x40080000;
    .text : ALIGN(16) { KEEP(*(.text.start)) *(.text*) }
    .rodata : ALIGN(16) { *(.rodata*) }
    .data : ALIGN(16) { *(.data*) }
    .bss : ALIGN(16) { *(.bss*) *(COMMON) }
}
