/* LabWired - conformance firmware linker script (STM32F103C8: 64K flash, 20K RAM).
 * NB: Rust/LLVM thumbv7m function symbols already carry the Thumb bit (odd
 * address), so vector entries use the symbol directly — adding 1 would clear it
 * and fault the core on the first instruction. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 64K
  RAM   : ORIGIN = 0x20000000, LENGTH = 20K
}
ENTRY(Reset)
SECTIONS
{
  .vector_table 0x08000000 :
  {
    LONG(0x20005000);       /*  0 initial SP — top of 20K SRAM */
    LONG(Reset);            /*  1 Reset */
    LONG(DefaultHandler);   /*  2 NMI */
    LONG(HardFaultHandler); /*  3 HardFault */
    LONG(DefaultHandler);   /*  4 MemManage */
    LONG(DefaultHandler);   /*  5 BusFault */
    LONG(DefaultHandler);   /*  6 UsageFault */
    LONG(0); LONG(0); LONG(0); LONG(0); /* 7-10 reserved */
    LONG(DefaultHandler);   /* 11 SVCall */
    LONG(DefaultHandler);   /* 12 Debug */
    LONG(0);                /* 13 reserved */
    LONG(DefaultHandler);   /* 14 PendSV */
    LONG(DefaultHandler);   /* 15 SysTick */
  } > FLASH

  .text :
  {
    *(.text*)
    *(.rodata*)
  } > FLASH

  /DISCARD/ : { *(.ARM.exidx*) *(.ARM.extab*) }
}
