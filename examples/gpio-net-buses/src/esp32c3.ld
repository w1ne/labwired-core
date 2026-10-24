/* ESP32-C3 / ESP32-C6 bare ELF: code in the flash cache window (loaded by
 * address), no .data/.bss (SRAM is used by address), stack set in _start. */
ENTRY(_start)
MEMORY {
  FLASH (rx) : ORIGIN = 0x42000000, LENGTH = 4M
}
SECTIONS {
  .text : { KEEP(*(.text.start)) *(.text*) *(.rodata*) *(.srodata*) } > FLASH
  /DISCARD/ : { *(.riscv.attributes) *(.comment) *(.note*) *(.eh_frame*) }
}
