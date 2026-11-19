ENTRY(_start)

SECTIONS
{
    . = 0x40000000;
    .text : { *(.text.start) *(.text*) }
    .rodata : { *(.rodata*) }
    /DISCARD/ : { *(.comment) *(.note*) }
}
