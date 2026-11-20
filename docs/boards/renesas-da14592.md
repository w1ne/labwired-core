# Renesas DA14592 validation status

## Current measurement (2026-11-09)

The imported renesas-da14592 system booted `firmware-renesas-da14592-smoke`
on the main Cortex-M33 for **100,000 instructions** and passed **6/6 smoke
assertions**. The firmware runs ten register checks and prints one letter
per check; the UART0 capture is `DA14592 checks PPPPPPPPPP DONE`, and the
summary word at 0x20017F00 is 0xDA1403FF. UART1 prints one line. P0.5 shows
6 edges. The result, UART capture, JUnit assertions, snapshot and build/run
logs are under
[`docs/coverage/renode-onboarding/metrics/renesas-da14592/`](../coverage/renode-onboarding/metrics/renesas-da14592/).

Checks (source: `crates/firmware-renesas-da14592-smoke/src/main.rs`):

0-1. UART0/UART1 LSR reset to THRE | TEMT (0x60).
2. P0_00_MODE and P1_15_MODE reset to 0x200 (input, pull-down); P0_DATA
   and P1_DATA read 0.
3. P0.5 as output: P0_SET drives it high, P0_SET reads 0, P0_RESET drives
   it low. P0.6 (still an input with pull-down) ignores the SET bit.
4. P1.2 as output through P1_SET/P1_RESET; P0_DATA is not touched.
5. P0.3 input reads 1 with PUPD pull-up and 0 with pull-down.
6. P0_WEAK_CTRL and P1_WEAK_CTRL are separate 16-bit words.
7. SRAM at 0x20000000 and its 0x800000 view are one store.
8. The ROM view at 0x900000 shows the vector table at 0x0.
9. eFlash data at 0x31000000 is seen at 0xA00000; the last SRAM word
   (0x20017FFC) holds a value.

## Declared surface

Sources: Renode `platforms/cpus/renesas-da14592.repl` (memory map, instance
bases), Renode `RenesasDA14_GPIO.cs` (GPIO register map) and Zephyr
`drivers/gpio/gpio_smartbond.c` (driver use of DATA/SET/RESET/MODE).

| Block | Base | Model |
|---|---|---|
| ROM 288 KiB | 0x00000000, mirror 0x00900000 | memory |
| SRAM 96 KiB | 0x20000000, mirror 0x00800000 | memory |
| eFlash data 0x40800 B | 0x31000000, mirror 0x00A00000 | memory |
| uart0 / uart1 | 0x50020000 / 0x50020100 (256 B each) | `renesasda14_uart` (DW APB 16550), polled |
| gpio (P0 + P1) | 0x50020600 (512 B) | `gpio` profile `da14` |

The GPIO model takes one instance for both ports. Pad `16 * port + n` is
Pport.n, so P1.2 is pad 18 (`gpio:18`). Renode's RESET-register handler ORs
`~value` into the latch; the model clears the written bits, as Zephyr's
`gpio_smartbond_port_clear_bits_raw` expects. PID != 0 reports the pad as
an alternate function; that PID 0 is GPIO is an assumption from DA1469x
(no DA1459x source checked). The DAP adapter does not bind board I/O to
this block (one offset pair cannot address both ports).

Not declared (no model in this engine): the auxiliary Cortex-M0+ CMAC core,
GPT timers 1-4, system watchdog, GPADC, DMA, DesignWare I2C, SPI, clock
generation and XTAL32M registers, general-purpose registers, QSPI and
eFlash controllers, pad latches, wake-up controller, and the CHIP_ID tags.
UART and GPIO interrupts are not modelled, so no NVIC lines are declared.

## Negative controls

Under
[`attempt-history/renesas-da14592/`](../coverage/renode-onboarding/attempt-history/renesas-da14592/):

- old descriptor (UART0 only, no mirrors): BusFault at 0x00800100 (SRAM
  low view), 1/6. The UART1 and GPIO accesses before it did not fault,
  because UART0's default window covered them. This round gives both UARTs
  256-byte windows.
- no GPIO block (256-byte UART windows): BusFault at 0x50020618
  (P0_00_MODE), 1/6.
- SRAM/ROM/eFlash-data low views as separate memories: checks 7-9 fail,
  4/6, summary 0xDA14007F.
