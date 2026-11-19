ENTRY(_start)

SECTIONS
{
    . = 0x00000000;
    .vectors : { KEEP(*(.vectors)) }
    .text : { *(.text.start) *(.text*) }
    .rodata : { *(.rodata*) }
    /DISCARD/ : { *(.comment) *(.note*) }
}
