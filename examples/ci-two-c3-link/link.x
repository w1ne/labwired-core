/* Bare ELF layout for the C3: code in IRAM, data in DRAM. The simulator loads
   this directly (fast boot), so there is no bootloader or flash image. */
MEMORY {
  IRAM (rwx) : ORIGIN = 0x4037C000, LENGTH = 384K
  DRAM (rw)  : ORIGIN = 0x3FC80000, LENGTH = 256K
}
ENTRY(_start)
SECTIONS {
  .text : { KEEP(*(.text._start)) *(.text .text.*) } > IRAM
  .rodata : { *(.rodata .rodata.*) } > IRAM
  .data : { *(.data .data.*) *(.sdata .sdata.*) } > DRAM
  .bss (NOLOAD) : { *(.bss .bss.*) *(.sbss .sbss.*) *(COMMON) } > DRAM
  _stack_top = ORIGIN(DRAM) + LENGTH(DRAM);
  /DISCARD/ : { *(.eh_frame*) *(.comment) }
}
