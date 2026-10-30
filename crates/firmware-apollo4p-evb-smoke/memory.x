/* Apollo4 Plus EVB (AMAP42KP-KBR): 2 MiB MRAM @ 0x0, the first 96 KiB
 * (0x0-0x17FFF) owned by the secure bootloader, so the application image
 * starts at 0x0001_8000 (Zephyr ambiq_apollo4p.dtsi flash0 <0x18000 0x1e8000>).
 * RAM: the 384-KiB TCM at 0x1000_0000 (am_reg_base_addresses.h
 * TCM_BASEADDR / TCM_MAX_SIZE); the smoke needs nothing beyond it. */

MEMORY
{
  FLASH : ORIGIN = 0x00018000, LENGTH = 2048K - 96K
  RAM   : ORIGIN = 0x10000000, LENGTH = 384K
}
