/* STM32F303RET6 main flash and SRAM. The separate CCM SRAM is not used. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 512K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}
