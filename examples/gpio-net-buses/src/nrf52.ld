/* nRF52840: flash at 0x00000000 with the vector table first. SRAM
 * 0x20000000..0x20001000 is left to the result block and the EasyDMA buffers
 * the firmware uses by address (EasyDMA reaches RAM only); .bss starts above
 * it. */
ENTRY(reset)
MEMORY {
  FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 1024K
  RAM (rwx) : ORIGIN = 0x20001000, LENGTH = 64K
}
SECTIONS {
  .text : { KEEP(*(.vectors)) *(.text*) *(.rodata*) } > FLASH
  .bss (NOLOAD) : { *(.bss*) *(COMMON) *(.data*) } > RAM
  /DISCARD/ : { *(.ARM.exidx*) *(.comment) *(.note*) }
}
