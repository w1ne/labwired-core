# Required Documents — CC2538DK

Authoritative sources used by this example (retrieved 2026-10-30):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| SWRU319C | CC2538 System-on-Chip Solution for 2.4-GHz IEEE 802.15.4 and ZigBee/ZigBee IP Applications — User's Guide (April 2012, rev. May 2013) | Table 4-1 memory map (every base address); Table 5-2 interrupt numbers; Table 7-3 SYS_CTRL registers and reset values (CLOCK_CTRL, CLOCK_STA, RCGCUART …); §8.6 / Tables 8-2..8-6 flash size and CCA layout; §8.9 SRAM retention; §9.1.1 / Table 9-1 IOC signal select; §9.2 / Table 9-2 GPIO (masked GPIO_DATA, DIR, AFSEL, reset pulls); Table 9-7 IOC registers; §18.4.2 baud-rate formula; §18.5 UART bring-up order; Table 18-4 UART0 registers |
| SWRS096 | CC2538 datasheet | Device table (CC2538SF53: 512 KB flash, 32 KB RAM); §5.18 DC characteristics (input thresholds) |
| Contiki-NG `arch/platform/cc2538dk/dev/board.h`, `leds-arch.c`, `board-buttons.c` | Board support (develop branch) | LED1-4 on PC0-PC3 active high, PC0 shared with the USB pull-up, keys PA3/PC4-PC7 with pull-up and negative logic, UART0 PA0 RX / PA1 TX |
| RIOT `boards/cc2538dk/include/board.h`, `periph_conf.h` | Board support (master) | Cross-check of the LED map (red/yellow/green/orange), UART0 PA0/PA1, CCA backdoor on PA3 active low |

The chip descriptor (`configs/chips/cc2538sf53.yaml`) carries the same values
transcribed with its own citations; this package does not restate them.

Not consulted: the SmartRF06 EB user's guide (SWRU321). The board wiring is
taken from the two OS ports above, which agree with each other.

Open item: SYS_CTRL gating, the IOC, the ROM and the unmodelled blocks are the
explicit L1 boundary, not documentation gaps.
