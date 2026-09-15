/* Sizes match configs/chips/stm32f401.yaml (the simulator's wiring). */

MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 512K
  RAM : ORIGIN = 0x20000000, LENGTH = 96K
}
