/* FRDM-K64F (MK64FN1M0VLL12): 1 MiB program flash @ 0x00000000,
 * 192 KiB SRAM_U @ 0x20000000 (SRAM_L at 0x1FFF0000 is not used by the smoke;
 * the vector table and stack live in SRAM_U). */

MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 192K
}
