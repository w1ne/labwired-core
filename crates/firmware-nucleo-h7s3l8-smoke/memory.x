/* STM32H7S3L8: 64 KiB user flash and 64 KiB DTCM used by this smoke. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 64K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}
