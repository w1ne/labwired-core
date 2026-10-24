ENTRY(Reset_Handler)
MEMORY
{
    FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 128K
    RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 128K
}
SECTIONS
{
    .text : { KEEP(*(.vectors)) *(.text*) *(.rodata*) } > FLASH
    .bss (NOLOAD) : { *(.bss*) *(COMMON) } > RAM
    _stack_top = ORIGIN(RAM) + LENGTH(RAM);
}
