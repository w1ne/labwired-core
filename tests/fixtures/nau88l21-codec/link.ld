MEMORY { FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 64K }
ENTRY(reset)
SECTIONS {
  .text : { KEEP(*(.vectors)) *(.text*) } > FLASH
}
