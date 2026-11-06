ENTRY(Reset_Handler)

MEMORY
{
    M33_CODE_SRAM (rx)  : ORIGIN = 0x00010000, LENGTH = 64K
    M33_DATA_SRAM (rwx) : ORIGIN = 0x20010000, LENGTH = 64K
}

__stack_top = ORIGIN(M33_DATA_SRAM) + LENGTH(M33_DATA_SRAM);

SECTIONS
{
    .text ORIGIN(M33_CODE_SRAM) : ALIGN(4)
    {
        KEEP(*(.text.Reset_Handler))
        *(.text*)
        *(.rodata*)
    } > M33_CODE_SRAM

    .data : ALIGN(4) { *(.data*) } > M33_DATA_SRAM AT > M33_CODE_SRAM
    .bss (NOLOAD) : ALIGN(4) { *(.bss*) *(COMMON) } > M33_DATA_SRAM
}
