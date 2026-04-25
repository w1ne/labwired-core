/* Minimal linker script for hand-asm Xtensa oracle fixtures.
 * Places .text.entry at 0x40370000 — the start of ESP32-S3 IRAM (Internal
 * SRAM 0 instruction-fetch window; TRM v1.4 §3.3.11).
 */

ENTRY(_start)

MEMORY {
    iram (rwx) : ORIGIN = 0x40370000, LENGTH = 0x10000
}

SECTIONS {
    .text 0x40370000 : {
        *(.text.entry)
        *(.text*)
    } > iram
}
