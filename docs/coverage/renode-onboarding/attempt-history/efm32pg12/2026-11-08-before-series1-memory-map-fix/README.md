# EFM32PG12 before the round-387 Series 1 memory-map fix

Metrics and logs as they stood before flash.base moved from 0x08000000 to 0x0
(Renode efm32xg1x/efm32pg12.repl maps flash at 0x00000000) and USART0 moved
from 0x4000C000 (I2C0 on silicon) to 0x40010000.
