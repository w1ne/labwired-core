/* NUCLEO-L552ZE-Q (STM32L552ZET6Q): 512 KiB flash @ 0x08000000,
 * 192 KiB SRAM1 @ 0x20000000 (the extra 64 KiB SRAM2 at 0x20030000 is not
 * used by the smoke; the vector table/stack live in SRAM1). */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 512K
  RAM   : ORIGIN = 0x20000000, LENGTH = 192K
}
