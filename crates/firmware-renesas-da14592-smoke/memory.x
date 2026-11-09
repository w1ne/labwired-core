/* DA14592 main-core map from Renode renesas-da14592.repl: ROM 0x48000 at 0,
   SRAM 0x18000 at 0x20000000. The top 256 bytes of SRAM hold the smoke
   summary and the SRAM-top check, so the stack starts below them. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 288K
  RAM   : ORIGIN = 0x20000000, LENGTH = 96K - 256
}
