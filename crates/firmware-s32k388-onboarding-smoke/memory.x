/* Linker ranges match the narrowed imported S32K388 execution profile. */
MEMORY
{
  FLASH : ORIGIN = 0x00400000, LENGTH = 8192K
  RAM   : ORIGIN = 0x20400000, LENGTH = 768K
}
