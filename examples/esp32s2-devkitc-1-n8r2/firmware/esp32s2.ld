ENTRY(main)

MEMORY
{
    FLASH (rx)  : ORIGIN = 0x40080000, LENGTH = 8M
    DRAM  (rw)  : ORIGIN = 0x3FFB0000, LENGTH = 320K
}

SECTIONS
{
    .text : ALIGN(4)
    {
        *(.literal .literal.*)
        *(.text .text.*)
        *(.rodata .rodata.*)
        . = ALIGN(4);
    } > FLASH

    .data : ALIGN(4)
    {
        *(.data .data.*)
        . = ALIGN(4);
    } > DRAM AT > FLASH

    .bss (NOLOAD) : ALIGN(4)
    {
        *(.bss .bss.* COMMON)
        . = ALIGN(4);
    } > DRAM

    /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
}
