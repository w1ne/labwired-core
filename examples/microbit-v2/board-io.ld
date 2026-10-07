/* SPDX-License-Identifier: MIT */
ENTRY(reset)
SECTIONS {
    . = 0;
    .vectors : { KEEP(*(.vectors)) }
    . = 0x100;
    .text : { *(.text*) *(.rodata*) }
}
