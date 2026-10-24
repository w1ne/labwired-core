ENTRY(_start)
MEMORY
{
    FLASH (rx) : ORIGIN = 0x80000000, LENGTH = 128K
    RAM (rwx)  : ORIGIN = 0x80020000, LENGTH = 64K
}
SECTIONS
{
    .text : { KEEP(*(.text.start)) *(.text*) *(.rodata*) *(.srodata*) } > FLASH
    .bss (NOLOAD) : { *(.bss*) *(.sbss*) *(COMMON) } > RAM
    _stack_top = ORIGIN(RAM) + LENGTH(RAM);
}
