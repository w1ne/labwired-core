/* ESP32-C3 (rv32imc) linker script for the LabWired sim.
 *
 * Matches the chip yaml memory map (configs/chips/esp32c3.yaml):
 *   FLASH (IROM) : 0x42000000, 4 MB  — code + rodata (XIP, loaded by paddr)
 *   RAM          : 0x3FC80000, 400 KB — data + bss + stack
 *
 * The simulator loads each PT_LOAD segment by its physical address (LMA), so
 * .data is placed in RAM (VMA) but loaded from flash (LMA); startup copies it.
 */
ENTRY(_start)

MEMORY
{
  FLASH (rx)  : ORIGIN = 0x42000000, LENGTH = 4M
  RAM   (rwx) : ORIGIN = 0x3FC80000, LENGTH = 400K
}

_stack_top = ORIGIN(RAM) + LENGTH(RAM);

SECTIONS
{
  .text :
  {
    KEEP(*(.init))
    *(.text*)
    *(.gnu.linkonce.t.*)
  } > FLASH

  .rodata :
  {
    *(.rodata*)
    *(.srodata*)
    *(.gnu.linkonce.r.*)
    . = ALIGN(4);
  } > FLASH

  /* Global-pointer anchor: centred so ±2 KB gp-relative addressing covers the
   * small-data sections. */
  . = ALIGN(8);
  PROVIDE(__global_pointer$ = . + 0x800);

  _sidata = LOADADDR(.data);
  .data :
  {
    . = ALIGN(4);
    _sdata = .;
    *(.data*)
    *(.sdata*)
    . = ALIGN(4);
    _edata = .;
  } > RAM AT > FLASH

  .bss (NOLOAD) :
  {
    . = ALIGN(4);
    _sbss = .;
    *(.sbss*)
    *(.bss*)
    *(COMMON)
    . = ALIGN(4);
    _ebss = .;
  } > RAM

  . = ALIGN(8);
  PROVIDE(end = .);
  PROVIDE(_end = .);

  /DISCARD/ : { *(.eh_frame*) *(.riscv.attributes) *(.comment) *(.note*) }
}
