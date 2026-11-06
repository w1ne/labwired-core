ENTRY(_start)

SECTIONS
{
    . = 0x20000000;
    .text : { *(.text.start) *(.text*) }
    .rodata : { *(.rodata*) }
    /DISCARD/ : { *(.comment) *(.note*) }
}
