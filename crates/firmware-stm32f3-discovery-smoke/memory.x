/* STM32F303VCT6 main SRAM. The separate CCM bank is not used by this smoke. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 40K
}
