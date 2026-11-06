ENTRY(_start)

SECTIONS
{
    . = 0x10000000;
    .text : { *(.text.start) *(.text*) }
    .rodata : { *(.rodata*) }
    /DISCARD/ : { *(.comment) *(.note*) *(.ARM.attributes) }
}
