MEMORY
{
  /* Leave the RP2040's 256-byte second-stage-bootloader slot at flash base. */
  FLASH : ORIGIN = 0x10000100, LENGTH = 0x1FFF00
  RAM : ORIGIN = 0x20000000, LENGTH = 264K
}
