/* ESP32-C2 (ESP8684H4X) bare-ELF layout: code in the IROM window, data and
 * stack in the DRAM view of SRAM (configs/chips/esp32c2.yaml). */
ENTRY(_start)
MEMORY
{
  FLASH (rx)  : ORIGIN = 0x42000000, LENGTH = 4M
  RAM   (rwx) : ORIGIN = 0x3FCA0000, LENGTH = 256K
}
SECTIONS
{
  .text : { KEEP(*(.init)) *(.text*) *(.rodata*) *(.srodata*) } > FLASH
  .bss (NOLOAD) : { *(.sbss*) *(.bss*) } > RAM
  /DISCARD/ : { *(.eh_frame*) *(.riscv.attributes) *(.comment) *(.note*) }
}
