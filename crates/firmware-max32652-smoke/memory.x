/* MAX32652 (Renode max32652.repl): 3 MiB flash at 0x10000000, 1 MiB SRAM at
   0x20000000. The top 4 KiB of SRAM hold the smoke summary and the SRAM-top
   probe, so the stack starts below them. */
MEMORY
{
  FLASH : ORIGIN = 0x10000000, LENGTH = 3M
  RAM   : ORIGIN = 0x20000000, LENGTH = 1020K
}
