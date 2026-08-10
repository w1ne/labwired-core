/* nRF52832 (QFAA) — Nordic nRF52832 Product Specification v1.8, §4 "Memory".
 * 512 KB flash at 0x00000000, 64 KB RAM at 0x20000000. This is the part the
 * simulator's configs/chips/nrf52832.yaml models, and the initial stack
 * pointer cortex-m-rt derives from ORIGIN(RAM) + LENGTH(RAM) must land inside
 * it: 0x20010000, not the 0x20040000 an nRF52840 build produces.
 */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 512K
  RAM : ORIGIN = 0x20000000, LENGTH = 64K
}
