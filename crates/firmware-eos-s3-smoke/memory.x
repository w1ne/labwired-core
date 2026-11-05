/* Renode EOS S3 M4 memory window: 512 KiB at 0, mirrored at 0x20000000. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 512K
  RAM   : ORIGIN = 0x20000000, LENGTH = 512K
}
