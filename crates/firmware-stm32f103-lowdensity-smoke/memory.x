/* STM32F103 low-density floor: x4 parts are 16 KB flash / 6 KB SRAM
   (DS5792 Table 2). Linking to the smallest orderable geometry lets one
   image boot on every F103 x4/x6 variant (C/R/T packages); the x6 parts'
   extra 16 KB flash / 4 KB SRAM go unused. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 16K
  RAM   : ORIGIN = 0x20000000, LENGTH = 6K
}
