ENTRY(_start)

MEMORY
{
    IROM (rx)  : ORIGIN = 0x40080000, LENGTH = 0x780000
    DROM (r)   : ORIGIN = 0x3F000000, LENGTH = 0x400000
    DRAM (rw)  : ORIGIN = 0x3FFB0000, LENGTH = 0x50000
}

SECTIONS
{
    .text : ALIGN(4)
    {
        *(.literal .literal.*)
        *(.text .text.*)
    } > IROM

    .rodata : ALIGN(4)
    {
        *(.rodata .rodata.*)
    } > DROM

    .data : ALIGN(4)
    {
        *(.data .data.*)
    } > DRAM AT > IROM

    .bss (NOLOAD) : ALIGN(4)
    {
        *(.bss .bss.* COMMON)
    } > DRAM
}
