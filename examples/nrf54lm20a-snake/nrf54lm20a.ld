/*
 * Linker script for the nRF54LM20A application core.
 *
 * Memory sizes are from the Zephyr devicetree
 * (dts/vendor/nordic/nrf54lm20_a_b.dtsi):
 *   cpuapp_rram: 0x00000000, 2036 KB   <- RRAM, not flash
 *   cpuapp_sram: 0x20000000,  511 KB
 *
 * Both numbers are odd and both are correct. 2036K is not "2 MB rounded", and
 * the SRAM is 511K rather than 512K because the top 1 KB is not in the
 * application core's window. Rounding either up puts the stack past the end of
 * memory, which a simulator will let you do for a while and silicon will not.
 */
MEMORY
{
  RRAM (rx)  : ORIGIN = 0x00000000, LENGTH = 2036K
  RAM  (rwx) : ORIGIN = 0x20000000, LENGTH = 511K
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
