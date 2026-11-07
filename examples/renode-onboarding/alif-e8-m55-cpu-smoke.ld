ENTRY(Reset_Handler)

MEMORY
{
    ITCM (rx)  : ORIGIN = 0x00000000, LENGTH = 256K
    DTCM (rwx) : ORIGIN = 0x20000000, LENGTH = 1M
}

__stack_top = ORIGIN(DTCM) + LENGTH(DTCM);

SECTIONS
{
    .isr_vector ORIGIN(ITCM) : ALIGN(4)
    {
        KEEP(*(.isr_vector))
    } > ITCM

    .text : ALIGN(4)
    {
        KEEP(*(.text.Reset_Handler))
        *(.text*)
    } > ITCM

    .rodata : ALIGN(4) { *(.rodata*) } > ITCM
    .data : ALIGN(4) { *(.data*) } > DTCM AT > ITCM
    .bss (NOLOAD) : ALIGN(4) { *(.bss*) *(COMMON) } > DTCM
}
