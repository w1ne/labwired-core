/* memory.x for firmware-arduino-nano-33-ble-smoke
 *
 * The nRF52840 maps 1 MiB of flash at 0x0 and 256 KiB of RAM at 0x20000000.
 */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 256K
}
