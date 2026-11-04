/* DA14592 main-core map from Renode: code flash at 0, SRAM at 0x20000000. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 288K
  RAM   : ORIGIN = 0x20000000, LENGTH = 96K
}
