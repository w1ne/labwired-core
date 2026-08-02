/*
 * Linker script for the nRF54L15 application core.
 *
 * Memory sizes are from the Zephyr devicetree (dts/vendor/nordic/nrf54l15.dtsi):
 *   cpuapp_rram: 0x00000000, 1524 KB   <- RRAM, not flash
 *   cpuapp_sram: 0x20000000,  256 KB
 *
 * 1524 KB is an odd number and it is correct; do not "round it up to 1.5 MB".
 */
MEMORY
{
  RRAM (rx)  : ORIGIN = 0x00000000, LENGTH = 1524K
  RAM  (rwx) : ORIGIN = 0x20000000, LENGTH = 256K
}

_estack = ORIGIN(RAM) + LENGTH(RAM);

ENTRY(Reset_Handler)

SECTIONS
{
  .isr_vector : {
    KEEP(*(.isr_vector))
  } > RRAM

  .text : {
    *(.text .text.*)
    *(.rodata .rodata.*)
    . = ALIGN(4);
  } > RRAM

  _sidata = LOADADDR(.data);

  .data : {
    . = ALIGN(4);
    _sdata = .;
    *(.data .data.*)
    . = ALIGN(4);
    _edata = .;
  } > RAM AT > RRAM

  .bss : {
    . = ALIGN(4);
    _sbss = .;
    *(.bss .bss.*)
    *(COMMON)
    . = ALIGN(4);
    _ebss = .;
  } > RAM

  /DISCARD/ : { *(.ARM.exidx*) }
}
