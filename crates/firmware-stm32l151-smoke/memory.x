/* STM32L151xD: 384 KiB flash, 48 KiB SRAM (the density Renode's stm32l151.repl maps). */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 384K
  RAM   : ORIGIN = 0x20000000, LENGTH = 48K
}
