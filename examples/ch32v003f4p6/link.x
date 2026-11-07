OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY
{
    FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 16K
    RAM (rw) : ORIGIN = 0x20000000, LENGTH = 2K
}

SECTIONS
{
    .text : { KEEP(*(.text._start)) *(.text .text.*) } > FLASH
    .rodata : { *(.rodata .rodata.*) } > FLASH
    .data : { *(.data .data.*) } > RAM AT> FLASH
    .bss : { *(.bss .bss.*) *(COMMON) } > RAM
}
