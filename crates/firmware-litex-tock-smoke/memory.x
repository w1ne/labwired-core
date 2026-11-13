/* Executable flash and main SRAM windows from Renode's litex_tock.repl. */
OUTPUT_ARCH(riscv)
ENTRY(_start)

MEMORY
{
    FLASH (rx)  : ORIGIN = 0x20000000, LENGTH = 32M
    RAM   (rwx) : ORIGIN = 0x01000000, LENGTH = 8K
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
        *(.sdata .sdata.*)
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
