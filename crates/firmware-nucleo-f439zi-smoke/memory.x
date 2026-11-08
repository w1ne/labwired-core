/* STM32F439ZIT6 main flash and SRAM. CCM and backup SRAM are not used. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 2M
  RAM   : ORIGIN = 0x20000000, LENGTH = 192K
}
