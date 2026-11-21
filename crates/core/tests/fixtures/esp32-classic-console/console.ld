ENTRY(_start)
MEMORY {
  iram (rx) : ORIGIN = 0x40080000, LENGTH = 128K
  dram (rw) : ORIGIN = 0x3FFB0000, LENGTH = 192K
}
SECTIONS {
  .text : { *(.literal .text .text.*) } > iram
  .data : { *(.data .data.* .rodata .rodata.*) } > dram
}
