ENTRY(_start)

SECTIONS
{
    . = 0x00000000;
    .text : { *(.text.start) *(.text*) }
    .rodata : { *(.rodata*) }
    /DISCARD/ : { *(.comment) *(.note*) *(.ARM.attributes) }
}
