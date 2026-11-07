ENTRY(_start)

MEMORY {
  FLASH (rx) : ORIGIN = 0x42000000, LENGTH = 4M
}

SECTIONS {
  .text : { KEEP(*(.text.start)) *(.text*) *(.rodata*) *(.srodata*) } > FLASH
  /DISCARD/ : { *(.riscv.attributes) *(.comment) *(.note*) *(.eh_frame*) }
}
