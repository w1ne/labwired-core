/* STM32F303K8T6 main flash and SRAM. The separate CCM bank is not used. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 64K
  RAM   : ORIGIN = 0x20000000, LENGTH = 12K
}
