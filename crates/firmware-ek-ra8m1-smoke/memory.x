/* EK-RA8M1 (R7FA8M1AHECBD): code flash @ 0x02000000 (2 MiB; Zephyr exposes
 * 2016 KiB), user SRAM 896 KiB @ 0x22000000 (Zephyr ra8x1.dtsi; FSP
 * bsp_feature.h BSP_FEATURE_FLASH_CODE_FLASH_START). The option-setting
 * memory (OFS0/OFS1, 0x0300A100+) is a separate region that the image does
 * not program, so the reset defaults apply. */

MEMORY
{
  FLASH : ORIGIN = 0x02000000, LENGTH = 2016K
  RAM   : ORIGIN = 0x22000000, LENGTH = 896K
}
