# Required Documents — CC2538DK

Authoritative sources used by this example (retrieved 2026-10-30):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| SWRU319C | CC2538 System-on-Chip Solution for 2.4-GHz IEEE 802.15.4 and ZigBee/ZigBee IP Applications — User's Guide (April 2012, rev. May 2013) | Table 4-1 memory map (every base address); Table 5-2 interrupt numbers; Table 7-3 SYS_CTRL registers and reset values (CLOCK_CTRL, CLOCK_STA, RCGCUART …); §8.6 / Tables 8-2..8-6 flash size and CCA layout; §8.9 SRAM retention; §9.1.1 / Table 9-1 IOC signal select; §9.2 / Table 9-2 GPIO (masked GPIO_DATA, DIR, AFSEL, reset pulls); Table 9-7 IOC registers; §18.4.2 baud-rate formula; §18.5 UART bring-up order; Table 18-4 UART0 registers |
| SWRU319C (cont.) | same | §4 Table 4-1 ROM 0x0000_0000-0x0001_FFFF; §7.7.1 CLOCK_STA.SYNC_32K; §8.2.1 information page; FLASH_CTRL_DIECFG0.CHIPID (0xB964); §12 / Table 12-1 MAC timer MTCTRL; §13 / Table 13-1 sleep timer (ST0 latch, compare, STLOAD); Table 14-1 WDCTL; §23 radio: Table 23-9 XREG reset values, Table 23-10 SFR, §23.9.1 RXENABLE control, §23.9.2 192-µs turnaround, §23.10.3 RSSI offset 73 dB, Figure 23-18 / Table 23-4 main-FSM states, §23.14.9 command strobes; Table 5-2 SM Timer IRQ 145 |
| SWRU333A | CC2538 ROM User's Guide (April 2013, CC BY-ND 3.0) | §2.2 ROM start-up sequence and §2.2.1 valid-image test (the clean-room ROM's reset path); §3.2 utility API prototypes, arguments and return values (the routines are written from these, not from TI code) |
| SWRS096 | CC2538 datasheet | Device table (CC2538SF53: 512 KB flash, 32 KB RAM); §5.18 DC characteristics (input thresholds) |
| Contiki-NG `arch/platform/cc2538dk/dev/board.h`, `leds-arch.c`, `board-buttons.c` | Board support (develop branch) | LED1-4 on PC0-PC3 active high, PC0 shared with the USB pull-up, keys PA3/PC4-PC7 with pull-up and negative logic, UART0 PA0 RX / PA1 TX |
| RIOT `boards/cc2538dk/include/board.h`, `periph_conf.h` | Board support (master) | Cross-check of the LED map (red/yellow/green/orange), UART0 PA0/PA1, CCA backdoor on PA3 active low |
| Contiki-NG release/v4.9 `arch/cpu/cc2538/` (`dev/rom-util.h`, `ieee-addr.c`, `dev/cc2538-rf.c`, `rtimer-arch.c`, `dev/sys-ctrl.c`) | CPU port (c20b12cd) | the ROM API table address 0x48 and entry order; the primary IEEE address location 0x0028_0028 and its byte order; which RF/sleep-timer registers the stock firmware polls |

The chip descriptor (`configs/chips/cc2538sf53.yaml`) carries the same values
transcribed with its own citations; this package does not restate them.

Not consulted: the SmartRF06 EB user's guide (SWRU321). The board wiring is
taken from the two OS ports above, which agree with each other.

Open items: TI does not publish the information-page layout beyond the IEEE
address, nor the ROM's internals (only its interface). SYS_CTRL gating, the
IOC, RF reception, the watchdog and the stub blocks are the explicit model
boundary, not documentation gaps.
