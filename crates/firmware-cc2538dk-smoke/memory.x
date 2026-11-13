/* CC2538DK (CC2538SF53): 512 KiB flash @ 0x00200000, 32 KiB SRAM @ 0x20000000
 * (SWRU319C Table 4-1; Table 8-2 512 KB option 0x00200000-0x0027FFFF).
 *
 * The top 44 bytes of the last 2-KiB flash page are the Customer
 * Configuration Area the ROM boot code reads (SWRU319C §8.6, Tables 8-3 to
 * 8-5): page bytes 2004-2007 boot-loader backdoor, 2008-2011 image valid,
 * 2012-2015 vector-table address, 2016-2047 page/debug lock bits. Page 255
 * starts at 0x0027F800, so byte 2004 is 0x0027FFD4. FLASH stops short of it. */

MEMORY
{
  FLASH : ORIGIN = 0x00200000, LENGTH = 512K - 44
  CCA   : ORIGIN = 0x0027FFD4, LENGTH = 44
  RAM   : ORIGIN = 0x20000000, LENGTH = 32K
}

SECTIONS
{
  .flashcca :
  {
    KEEP(*(.flashcca));
  } > CCA
} INSERT AFTER .rodata;
