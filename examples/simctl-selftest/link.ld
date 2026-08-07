/* Minimal map for the ci-fixture Cortex-M3 board: flash at 0, RAM at 0x20000000. */
MEMORY
{
  FLASH (rx)  : ORIGIN = 0x00000000, LENGTH = 128K
  RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 128K
}

_stack_top = ORIGIN(RAM) + LENGTH(RAM);

SECTIONS
{
  .text : {
    KEEP(*(.isr_vector))
    *(.text*)
    *(.rodata*)
  } > FLASH

  .data : { *(.data*) } > RAM AT> FLASH
  .bss  : { *(.bss*) *(COMMON) } > RAM
}
