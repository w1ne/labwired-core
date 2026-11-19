/* Zolertia Firefly (Zoul module, CC2538SF53): 512 KiB flash @ 0x00200000,
 * 32 KiB SRAM @ 0x20000000 (SWRU319C Table 4-1; Table 8-2).
 *
 * The top 44 bytes of the last 2-KiB flash page are the Customer
 * Configuration Area (SWRU319C §8.6, Tables 8-3 to 8-5), as in the CC2538DK
 * smoke. The top 256 bytes of SRAM hold the smoke's summary word at
 * 0x20007F00, outside the stack. */

MEMORY
{
  FLASH : ORIGIN = 0x00200000, LENGTH = 512K - 44
  CCA   : ORIGIN = 0x0027FFD4, LENGTH = 44
  RAM   : ORIGIN = 0x20000000, LENGTH = 32K - 256
}

SECTIONS
{
  .flashcca :
  {
    KEEP(*(.flashcca));
  } > CCA
} INSERT AFTER .rodata;
