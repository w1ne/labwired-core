ENTRY(_start)

SECTIONS
{
    . = 0x3F000000;
    .text : ALIGN(4) { KEEP(*(.text.start)) *(.text*) }
    .rodata : ALIGN(4) { *(.rodata*) }
    .data : ALIGN(4) { *(.data*) }
    .bss : ALIGN(4) { *(.bss*) *(COMMON) }
}
