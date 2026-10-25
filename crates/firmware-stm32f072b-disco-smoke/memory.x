/* memory.x for firmware-stm32f072b-disco-smoke
 *
 * STM32F072RB: 128 KiB of flash at 0x08000000, 16 KiB of SRAM at 0x20000000.
 */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM   : ORIGIN = 0x20000000, LENGTH = 16K
}
