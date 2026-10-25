/* memory.x for firmware-sam4s-xplained-smoke
 *
 * ATSAM4S16C: 1 MiB of flash at 0x00400000, 128 KiB of SRAM at 0x20000000.
 */
MEMORY
{
  FLASH : ORIGIN = 0x00400000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 128K
}
