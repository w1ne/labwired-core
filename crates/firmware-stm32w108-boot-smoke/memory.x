/*
 * Linker ranges mirror the imported STM32W108 loader profile. This smoke
 * checks instruction execution in that profile; it does not validate the
 * physical memory capacity or peripheral behavior.
 */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 1024K
}
