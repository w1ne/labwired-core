/* Executable ROM and RAM windows from Renode's Tock VeeR EL2 platform. */
OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY
{
    FLASH (rx)  : ORIGIN = 0x20000000, LENGTH = 3M
    RAM   (rwx) : ORIGIN = 0x20600000, LENGTH = 3M
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

    .data : ALIGN(4)
    {
        *(.data .data.*)
    } > RAM AT> FLASH

    .bss (NOLOAD) : ALIGN(4)
    {
        *(.sbss .sbss.* .bss .bss.*)
        *(COMMON)
    } > RAM

    __stack_top = ORIGIN(RAM) + LENGTH(RAM);

    /DISCARD/ :
    {
        *(.eh_frame .eh_frame.*)
        *(.comment)
    }
}
