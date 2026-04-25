/* Linker script for CubeMX-style HAL firmware.
 *
 * Has a full Cortex-M4 vector table (16 system + 82 NVIC slots) and proper
 * .data / .bss sections. Reset_Handler must:
 *   1. Copy .data initialiser from FLASH to RAM
 *   2. Zero .bss
 *   3. Set up the FPU (CPACR.CP10/CP11)
 *   4. Branch to main
 *
 * SysTick_Handler increments the global `uwTick` so HAL_Delay() can poll it.
 */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 96K
}

ENTRY(Reset_Handler)

SECTIONS
{
  .isr_vector 0x08000000 :
  {
    KEEP(*(.isr_vector))
  } > FLASH

  .text :
  {
    *(.text*)
    *(.rodata*)
    _etext = .;
  } > FLASH

  .data : AT (_etext)
  {
    _sdata = .;
    *(.data*)
    _edata = .;
  } > RAM

  .bss :
  {
    _sbss = .;
    *(.bss*)
    *(COMMON)
    _ebss = .;
  } > RAM

  /DISCARD/ :
  {
    *(.ARM.exidx*)
    *(.note.gnu.build-id*)
  }
}
