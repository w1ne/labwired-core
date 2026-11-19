/* EFM32LG995F256: 256 KiB flash at 0x00000000 and 32 KiB SRAM (Renode
 * efm32lg995.repl). The top 4 KiB of SRAM is kept out of the linker's RAM
 * so the smoke's summary word can never be overwritten by the stack. */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 28K
}
