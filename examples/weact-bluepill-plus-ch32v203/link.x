OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY
{
    FLASH (rx)  : ORIGIN = 0x08000000, LENGTH = 64K
    RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 20K
}

SECTIONS
{
    .text : ALIGN(4)
    {
        KEEP(*(.text.entry))
        *(.text .text.*)
        *(.rodata .rodata.*)
    } > FLASH

    .data : ALIGN(4)
    {
        *(.data .data.*)
    } > RAM AT > FLASH

    .bss (NOLOAD) : ALIGN(4)
    {
        *(.bss .bss.*)
        *(COMMON)
    } > RAM
}

__stack_top = ORIGIN(RAM) + LENGTH(RAM);
