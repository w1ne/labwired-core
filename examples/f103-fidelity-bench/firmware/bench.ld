ENTRY(Reset)

/* STM32F103C8: 64 KB flash, 20 KB SRAM. rambug stores past this RAM. */
MEMORY
{
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 64K
  RAM  (rwx) : ORIGIN = 0x20000000, LENGTH = 20K
}

_estack = ORIGIN(RAM) + LENGTH(RAM);

SECTIONS
{
  .isr_vector : { KEEP(*(.isr_vector)) } > FLASH
  .text       : { *(.text*) *(.rodata*) } > FLASH
  _sidata = LOADADDR(.data);
  .data : { . = ALIGN(4); _sdata = .; *(.data*) . = ALIGN(4); _edata = .; } > RAM AT > FLASH
  .bss  : { . = ALIGN(4); _sbss = .; *(.bss*) *(COMMON) . = ALIGN(4); _ebss = .; } > RAM
  . = ALIGN(8);
  PROVIDE(end = .);
  PROVIDE(_end = .);
  /DISCARD/ : { *(.ARM.exidx*) *(.note.gnu.build-id*) }
}
