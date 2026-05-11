/* Minimal linker script for the F407 demo. Emits a full 16-entry
 * Cortex-M4 vector table — exceptions other than Reset all point to a
 * `default_handler` symbol that spins forever, so any unhandled
 * fault halts cleanly instead of decoding garbage as the handler PC.
 * The simulator's ELF loader picks up the `Reset` symbol as entry. */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 128K
}

ENTRY(Reset)

SECTIONS
{
  .vector_table 0x08000000 :
  {
    LONG(0x20020000);            /*  0 — Initial SP (top of 128 KB SRAM) */
    LONG(Reset + 1);             /*  1 — Reset */
    LONG(default_handler + 1);   /*  2 — NMI */
    LONG(default_handler + 1);   /*  3 — HardFault */
    LONG(default_handler + 1);   /*  4 — MemManage */
    LONG(default_handler + 1);   /*  5 — BusFault */
    LONG(default_handler + 1);   /*  6 — UsageFault */
    LONG(0);                     /*  7 — reserved */
    LONG(0);                     /*  8 — reserved */
    LONG(0);                     /*  9 — reserved */
    LONG(0);                     /* 10 — reserved */
    LONG(default_handler + 1);   /* 11 — SVCall */
    LONG(default_handler + 1);   /* 12 — DebugMonitor */
    LONG(0);                     /* 13 — reserved */
    LONG(default_handler + 1);   /* 14 — PendSV */
    LONG(default_handler + 1);   /* 15 — SysTick */
  } > FLASH

  .text :
  {
    *(.text*)
    *(.rodata*)
  } > FLASH

  /DISCARD/ :
  {
    *(.ARM.exidx*)
    *(.note.gnu.build-id*)
  }
}
