/* STM32L476RG: 1 MB flash @ 0x08000000, 96 KB SRAM1 @ 0x20000000. */
MEMORY
{
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM   (rw) : ORIGIN = 0x20000000, LENGTH = 96K
}

SECTIONS
{
  .vectors 0x08000000 : { KEEP(*(.vectors)) } > FLASH
  .text : { *(.text*) *(.rodata*) } > FLASH

  /* Results pinned at the start of RAM so SWD / the emulator read a fixed addr. */
  .results 0x20000000 (NOLOAD) : { KEEP(*(.results)) } > RAM
  .bss (NOLOAD) : { *(.bss*) *(COMMON) } > RAM

  /DISCARD/ : { *(.ARM.exidx*) *(.comment) }
}
