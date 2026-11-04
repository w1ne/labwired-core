/* Bounds follow Renode's s32k118.repl program and upper SRAM regions. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 0x5800
}
