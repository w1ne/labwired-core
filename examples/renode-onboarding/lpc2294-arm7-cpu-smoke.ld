ENTRY(_start)

SECTIONS
{
    . = 0x08000000;
    .text : { *(.text.start) *(.text*) }
    .rodata : { *(.rodata*) }
    /DISCARD/ : { *(.comment) *(.note*) }
}
