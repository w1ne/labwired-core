ENTRY(Reset_Handler)
MEMORY {
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 512K
  RAM (rwx) : ORIGIN = 0x20000000, LENGTH = 64K
}
SECTIONS {
  .isr_vector : { KEEP(*(.isr_vector)) } > FLASH
  .text : { *(.text*) *(.rodata*) . = ALIGN(4); } > FLASH
  .ARM.exidx : { *(.ARM.exidx*) } > FLASH
  _sidata = LOADADDR(.data);
  .data : { . = ALIGN(4); _sdata = .; *(.data*) . = ALIGN(4); _edata = .; } > RAM AT > FLASH
  .bss : { . = ALIGN(4); _sbss = .; *(.bss*) *(COMMON) . = ALIGN(4); _ebss = .; } > RAM
}
