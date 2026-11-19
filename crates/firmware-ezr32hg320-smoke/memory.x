/* EZR32HG320: 64 KiB flash at 0x00000000 and 8 KiB SRAM (Renode
 * ezr32hg320.repl). The top 1 KiB of SRAM is kept out of the linker's RAM
 * so the smoke's summary word can never be overwritten by the stack. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 64K
  RAM   : ORIGIN = 0x20000000, LENGTH = 7K
}
