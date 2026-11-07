ENTRY(_start)
MEMORY
{
  iram (rwx) : ORIGIN = 0x40370000, LENGTH = 128K
  dram (rw)  : ORIGIN = 0x3FC88000, LENGTH = 512K
}
SECTIONS
{
  .text : { *(.literal .literal.*) *(.text .text.*) } > iram
  .rodata : { *(.rodata .rodata.*) } > iram
  .data : { *(.data .data.*) } > dram
  .bss : { *(.bss .bss.* COMMON) } > dram
}
