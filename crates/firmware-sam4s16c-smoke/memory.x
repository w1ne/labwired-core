/* Bounded SAM4S16C execution windows used by the UART smoke. */
MEMORY
{
  FLASH : ORIGIN = 0x00400000, LENGTH = 1024K
  RAM   : ORIGIN = 0x20000000, LENGTH = 128K
}
