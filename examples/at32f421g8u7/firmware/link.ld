ENTRY(Reset_Handler)
MEMORY { FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 63K
         RAM (rwx) : ORIGIN = 0x20000000, LENGTH = 16K }
_estack = ORIGIN(RAM) + LENGTH(RAM);
SECTIONS {
 .isr_vector : { KEEP(*(.isr_vector)) } > FLASH
 .text : { *(.text*) *(.rodata*) } > FLASH
 .ARM.exidx : { *(.ARM.exidx*) } > FLASH
 _sidata = LOADADDR(.data);
 .data : { _sdata = .; *(.data*) _edata = .; } > RAM AT> FLASH
 .bss : { _sbss = .; *(.bss*) *(COMMON) _ebss = .; } > RAM
}
