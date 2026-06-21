/* STM32L476: 1 MB flash @ 0x08000000, 96 KB SRAM1 @ 0x20000000. */
ENTRY(Reset)
MEMORY
{
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM  (rwx) : ORIGIN = 0x20000000, LENGTH = 96K
}
_estack = ORIGIN(RAM) + LENGTH(RAM);
SECTIONS
{
  .isr_vector : { KEEP(*(.isr_vector)) } > FLASH
  .text       : { *(.text*) *(.rodata*) } > FLASH
  _sidata = LOADADDR(.data);
  .data : { . = ALIGN(4); _sdata = .; *(.data*) . = ALIGN(4); _edata = .; } > RAM AT > FLASH
  .bss  : { . = ALIGN(4); _sbss = .; *(.bss*) *(COMMON) . = ALIGN(4); _ebss = .; } > RAM
  /* Heap base for newlib _sbrk (malloc/snprintf). Heap grows toward the stack. */
  . = ALIGN(8);
  PROVIDE(end = .);
  PROVIDE(_end = .);
  /DISCARD/ : { *(.ARM.exidx*) *(.note.gnu.build-id*) }
}
