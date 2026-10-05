/* STM32F401: SRAM 0x20000000..0x20001000 is left to the result block the
 * firmware writes by address; .bss starts above it. */
ENTRY(reset)
MEMORY {
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 512K
  RAM (rwx) : ORIGIN = 0x20001000, LENGTH = 60K
}
SECTIONS {
  .text : { KEEP(*(.vectors)) *(.text*) *(.rodata*) } > FLASH
  .bss (NOLOAD) : { *(.bss*) *(COMMON) *(.data*) } > RAM
}
