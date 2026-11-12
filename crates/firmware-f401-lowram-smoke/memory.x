/* STM32F401 floor: xB parts are 128 KB flash, xB/xC parts 64 KB SRAM
   (DS9716 Table 2). cortex-m-rt puts the initial SP at the end of RAM,
   0x2001_0000, which is valid on every F401 variant; the xD/xE parts'
   extra 32 KB SRAM goes unused. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}
