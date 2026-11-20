OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY
{
    FLASH (rx) : ORIGIN = 0x1A000000, LENGTH = 8K
}

SECTIONS
{
    .text : ALIGN(4)
    {
        KEEP(*(.text.entry))
        *(.text .text.*)
    } > FLASH

    .rodata : ALIGN(4)
    {
        *(.rodata .rodata.*)
    } > FLASH

    /DISCARD/ :
    {
        *(.eh_frame .eh_frame.*)
        *(.comment)
    }
}
