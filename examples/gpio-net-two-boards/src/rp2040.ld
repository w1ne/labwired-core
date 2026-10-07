/* RP2040: flash is mapped directly at 0x10000000; the simulator starts at the
 * vector table after the 256-byte boot2 slot (reset_vector_offset 0x100). The
 * firmware keeps no .data/.bss: SRAM is used by address. */
ENTRY(reset)
MEMORY {
  FLASH (rx) : ORIGIN = 0x10000100, LENGTH = 2M - 256
}
SECTIONS {
  .text : { KEEP(*(.vectors)) *(.text*) *(.rodata*) } > FLASH
  /DISCARD/ : { *(.ARM.exidx*) *(.comment) *(.note*) }
}
