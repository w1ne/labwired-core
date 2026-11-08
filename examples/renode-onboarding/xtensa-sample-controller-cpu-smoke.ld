ENTRY(_start)

SECTIONS
{
    . = 0x50000000;
    .text : { *(.text.start) *(.literal*) *(.text*) }
    /DISCARD/ : { *(.comment) *(.note*) *(.xtensa.info) *(.xt.*) }
}
