/* EK-RA6M5 (R7FA6M5BH3CFC): 2 MiB code flash @ 0x00000000, 512 KiB SRAM @
 * 0x20000000 (Zephyr r7fa6m5bh3cfc.dtsi / r7fa6m5xh.dtsi; R7FA6M5BH.h).
 * The option-setting memory (OFS0/OFS1, 0x0100A100+) is a separate region
 * that the image does not program, so the reset defaults apply. */

MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 2M
  RAM   : ORIGIN = 0x20000000, LENGTH = 512K
}
