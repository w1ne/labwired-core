/* Linker script for the LOOPBACK differential firmware.
 *
 * Identical to ../minimal.ld EXCEPT RAM LENGTH = 20K. The chip *model* exposes
 * 128KB SRAM, but the physical silicon (STM32F103C8, IDCODE 0x20036410) has
 * only 20KB. ../minimal.ld places _estack at 0x20020000 (top of the 128KB
 * model) which is past the end of physical RAM on the real C8 — the first stack
 * push in Reset_C HardFaults before main() runs. A 20K stack top (0x20005000)
 * is valid on BOTH the real board and the 128KB sim, so the same ELF is truly
 * differential. The result block @ 0x20000400 is well inside 20K either way.
 */
ENTRY(Reset)

MEMORY
{
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 1024K
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
