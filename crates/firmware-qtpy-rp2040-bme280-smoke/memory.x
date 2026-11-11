/* Cortex-M vectors follow the RP2040's 256-byte second-stage boot slot. */
MEMORY
{
  FLASH : ORIGIN = 0x10000100, LENGTH = 0x1FFF00
  RAM : ORIGIN = 0x20000000, LENGTH = 264K
}
