/* Renode MIMXRT798S: XIP1 @ 0x08000000, CPU0 non-secure SRAM @ 0x20000000.
 * The first 0x100 bytes hold the RESULT/PASS words the smoke asserts, so
 * .data/.bss start above them. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 128M
  RAM   : ORIGIN = 0x20000100, LENGTH = 0x580000 - 0x100
}
