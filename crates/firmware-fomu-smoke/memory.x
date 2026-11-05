OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY
{
    ROM (rx) : ORIGIN = 0x00000000, LENGTH = 8K
    RAM (rw) : ORIGIN = 0x10000000, LENGTH = 128K
}

SECTIONS
{
    .text : ALIGN(4)
    {
        KEEP(*(.text.entry))
        *(.text .text.*)
    } > ROM

    .rodata : ALIGN(4)
    {
        *(.rodata .rodata.*)
    } > ROM

    /DISCARD/ :
    {
        *(.eh_frame .eh_frame.*)
        *(.comment)
    }
}
