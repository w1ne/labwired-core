/* memory.x for firmware-stm32u575i-ev-smoke
 *
 * STM32U575AII6Q: 2 MiB of flash at 0x08000000, 768 KiB main SRAM at
 * 0x20000000. SRAM4 (16 KiB at 0x28000000) and backup SRAM are not used by
 * this smoke.
 */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 2048K
  RAM   : ORIGIN = 0x20000000, LENGTH = 768K
}
