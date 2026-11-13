/* STM32F072xB: 128 KiB flash, 16 KiB SRAM (the density Renode's stm32f072.repl maps). */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM   : ORIGIN = 0x20000000, LENGTH = 16K
}
