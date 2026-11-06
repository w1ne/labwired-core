ENTRY(_start)

MEMORY
{
  iram (rx)  : ORIGIN = 0x40370000, LENGTH = 512K
  dram (rw)  : ORIGIN = 0x3FC88000, LENGTH = 512K
}

SECTIONS
{
  .entry : ALIGN(4) { KEEP(*(.entry)) } > iram
  .text : ALIGN(4) { *(.text .text.*) *(.rodata .rodata.*) } > iram
  .data : ALIGN(4) { *(.data .data.*) } > dram AT > iram
  .bss (NOLOAD) : ALIGN(4) { *(.bss .bss.* COMMON) } > dram
  . = ALIGN(16);
  _stack_top = ORIGIN(dram) + LENGTH(dram);
}
