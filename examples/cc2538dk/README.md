# CC2538DK — LabWired example

The [CC2538DK](https://www.ti.com/tool/CC2538DK) development kit: a
**CC2538SF53** evaluation module (Cortex-M3, 512 KiB flash, 32 KiB SRAM,
32 MHz max, 2.4-GHz IEEE 802.15.4 radio) on a SmartRF06 evaluation board. This
is the **UART+LED L1 slice** — the radio and the rest of the SoC are declared
stubs.

## What runs here

A bare-register smoke firmware (`crates/firmware-cc2538dk-smoke`) that:

- gates UART0 on (`SYS_CTRL_RCGCUART`),
- routes **PA1** to UART0 TXD (`IOC_PA1_SEL = 0`, `IOC_PA1_OVER = OE`) and
  **PA0** to UART0 RX (`IOC_UARTRXD_UART0 = 0`), and hands both pins to the
  peripheral (`GPIO_AFSEL`),
- reads **`SYS_CTRL_CLOCK_STA`** for the reset clock (16-MHz HF-RC,
  `SYS_DIV = 001`) and derives the **UART0** divisors from it —
  `IBRD = 8`, `FBRD = 44` for 115200 8N1 (SWRU319C §18.4.2),
- walks **LED2 yellow PC1, LED3 green PC2, LED4 orange PC3** (active high)
  with single address-masked `GPIO_DATA` stores and prints both the pin and
  the whole LED nibble it read back,
- prints the five keys' pad levels (released = 1 on the reset pull-up), then
  heartbeats.

LED1 (red, PC0) is **not driven**: PC0 is also the CC2538 USB D+ pull-up.

The image carries a real Customer Configuration Area at `0x0027_FFD4`
(image valid, vector table `0x0020_0000`, SELECT-key backdoor), so the same
ELF would boot through the CC2538 ROM on silicon. The simulator fast-boots it.

## Build and run

```bash
# from the repository root
cargo build -p firmware-cc2538dk-smoke --release --target thumbv7m-none-eabi

./target/release/labwired test \
  --script examples/cc2538dk/io-smoke.yaml \
  --output-dir out/cc2538dk-smoke --no-uart-stdout
```

Interactive run (UART to stdout):

```bash
./target/release/labwired \
  --firmware target/thumbv7m-none-eabi/release/firmware-cc2538dk-smoke \
  --system configs/systems/cc2538dk.yaml \
  --max-steps 400000
```

Expected output (first beats):

```text
CC2538DK SMOKE
cpu=cortex-m3 uart=uart0
clk osc=hfrc16 sys_div=1 io_div=1 sysclk=16000000
uart0 rcgc=1 ibrd=8 fbrd=44
led=yellow on pin=1 leds=02
led=yellow off pin=0 leds=00
led=green on pin=1 leds=04
led=green off pin=0 leds=00
led=orange on pin=1 leds=08
led=orange off pin=0 leds=00
keys select=1 left=1 right=1 up=1 down=1
hb=0 yellow=1 green=0 orange=0
...
```

## Board facts used

| Item | Value | Source |
| :--- | :--- | :--- |
| MCU | CC2538SF53 (512 KB flash, 32 KB RAM) | SWRS096 device table |
| Core / clock | Cortex-M3, 32 MHz max; 16-MHz HF-RC at reset (`SYS_DIV 001`) | SWRU319C Table 7-3 |
| Flash / RAM | 512 KiB @ 0x00200000 / 32 KiB @ 0x20000000 (upper 16 KiB retained) | SWRU319C Table 4-1, Table 8-2, §8.9 |
| ROM | 0x00000000 (boot ROM; not modelled) | SWRU319C Table 4-1, §8.6.1 |
| LEDs | LED1 red PC0, LED2 yellow PC1, LED3 green PC2, LED4 orange PC3, active high | Contiki-NG `cc2538dk` dev/board.h + leds-arch.c; RIOT `cc2538dk` board.h |
| Keys | SELECT PA3, LEFT PC4, RIGHT PC5, UP PC6, DOWN PC7, active low, pull-up | Contiki-NG `cc2538dk` dev/board.h + board-buttons.c |
| Console UART | UART0 PA0 (RX) / PA1 (TX), 115200 8N1 | Contiki-NG dev/board.h; RIOT periph_conf.h |

## Explicitly not claimed

- **SYS_CTRL clock gating** has no effect: the gates are stored and read back,
  but no peripheral is gated by them. Oscillator start-up timing, the 32-kHz
  sync, power modes and software reset are not modelled.
- **IOC** pin muxing and pad overrides are a stub. The firmware writes them as
  §18.5 requires, but the console model does not depend on them.
- **GPIO interrupts** (`GPIO_RIS`/`GPIO_MIS`) are never raised; the AFSEL commit
  lock is not enforced.
- **RF core, AES/PKA, flash controller, SSI0/1, I2C, GPTimer0-3, USB, uDMA,
  WDT/sleep timer, SoC ADC** are declared stubs.
- The **boot ROM** is not present; the engine enters the ELF at its flash
  vector table, which is what the ROM does for a valid CCA image.

See `VALIDATION.md` for the tier table.
