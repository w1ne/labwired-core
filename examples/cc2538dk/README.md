# CC2538DK — LabWired example

The [CC2538DK](https://www.ti.com/tool/CC2538DK) development kit: a
**CC2538SF53** evaluation module (Cortex-M3, 512 KiB flash, 32 KiB SRAM,
32 MHz max, 2.4-GHz IEEE 802.15.4 radio) on a SmartRF06 evaluation board. This
is the **UART+LED L1 slice**, plus **stock Contiki-NG v4.9 hello-world**
booting through a clean-room boot ROM with the RF core and sleep timer
modelled, the flash controller (Contiki-NG CFS/Coffee) and the AES/SHA-256
cryptoprocessor (the Contiki-NG crypto suite). PKA, SSI, I2C, GPTimers, USB,
uDMA and the ADC are declared stubs.

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
ELF would boot through the CC2538 ROM on silicon. The loader enters it at
that flash-base table directly.

### Stock Contiki-NG hello-world

`tests/fixtures/cc2538dk-contiki-hello.elf` is the **unmodified** upstream
Contiki-NG v4.9 `examples/hello-world` for `TARGET=cc2538dk` (rebuild:
`crates/firmware-cc2538dk-contiki/build.sh`). `contiki-hello.yaml` asserts its
banner (RPL Lite, sicslowpan, CSMA, channel 26), the link-layer address
`0012.4b00.0000.0001` it derives from the information page, and three
`Hello, world` lines (boot + two 10-s etimer periods). On the way it runs:

- the **ROM** (`crates/core/roms/cc2538/cc2538_rom.S`, written from the
  SWRU333A interface, not a TI dump): CCA check, VTOR = `0x0020_2000`, then
  `reset_handler`'s `.data` copy and `.bss` clear through the `0x48` API;
- `SYS_CTRL_CLOCK_STA.SYNC_32K` (`sys_ctrl_init` waits for one 32-kHz edge);
- the **RF core** (`random_init`: `ISRXON` → `RSSI_VALID` → `RFRND`, then the
  CSMA radio driver's channel/CCA/MAC-timer bring-up);
- the **sleep timer** count (rtimer, LPM durations) and SysTick (the etimer).
  The compare interrupt is not used by this firmware (the listening radio
  keeps LPM in PM0); `cc2538_sleep_timer_irq` covers it.

### Flash: the flash controller and Contiki-NG CFS/Coffee

The ROM's `PageErase`/`ProgramFlash` drive the FLASH_CTRL model
(`crates/core/src/peripherals/cc2538/flash_ctrl.rs`): 20-ms page erase, the
SWRU319C §8.3.1 `FCTL`/`FADDR`/`FWDATA` write sequence (20 µs per word, NOR
AND rule), CCA page-lock bits and the lock-bit page's `UPPER_PAGE_ACCESS`.

- `contiki-test-cfs.yaml` / `contiki-coffee.yaml`: the **unmodified**
  Contiki-NG v4.9 `examples/storage/cfs-coffee` `test-cfs` and
  `example-coffee` (a 256 KiB Coffee file system): `CFS test 1/2 completed
  with 0 errors`; write, read back, append and list a file.
- `flash-controller.yaml`: `crates/firmware-cc2538dk-contiki/flash-test`,
  the steps of Renode's `cc2538_flash_controller.robot` (memset write,
  pattern write, page erase, two-page erase). It reports `FLASH SIZE:
  0x80000`, what a CC2538SF53's `DIECFG0` gives; Renode prints `0x10000`
  because its model returns the instant-of-reset `DIECFG0`.

### Crypto: the AES/SHA-256 cryptoprocessor

The `cc2538_aes` model (`crates/core/src/peripherals/cc2538/aes.rs`, SWRU319C
§22.2) moves data with its own two-channel DMA, loads keys through the key
store and raises the level AES interrupt (IRQ 143). The **unmodified**
Contiki-NG v4.9 `examples/platform-specific/cc2538-common/crypto` suite runs
on it, each test from its own fixture, every vector printing `... OK`:

| script | vectors | Renode `cc2538_cryptoprocessor.robot` |
|---|---|---|
| `crypto-cbc.yaml` | 16 AES-CBC | "Should Handle CBC Mode" (superset) |
| `crypto-cbc-mac.yaml` | 16 AES-CBC-MAC | "Should Handle CBC MAC Mode" (superset) |
| `crypto-ctr.yaml` | 6 AES-CTR | "Should Handle CTR Mode" (superset) |
| `crypto-ccm.yaml` | 22 AES-CCM, incl. AAD-only and `mic_len` 0 | "Should Handle CCM Mode" (superset) |
| `crypto-ecb.yaml` | 16 AES-ECB | not in Renode's test |
| `crypto-gcm.yaml` | 8 + 8 AES-GCM | not in Renode's test |
| `crypto-sha256.yaml` | 9 SHA-256, intermediate digests resumed | not in Renode's test |

Renode's test loads a TI ROM dump and two flip-flop stubs; these run through
LabWired's clean-room ROM and the SYS_CTRL model.

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
| ROM | 0x00000000-0x0001FFFF; clean-room ROM: CCA boot, utility API at 0x48 | SWRU319C Table 4-1; SWRU333A §2.2, §3.2 |
| IEEE address | info page 0x00280028, fixed 00:12:4B:00:00:00:00:01 (not a real device's) | Contiki-NG `ieee-addr.h`/`ieee-addr.c` |
| LEDs | LED1 red PC0, LED2 yellow PC1, LED3 green PC2, LED4 orange PC3, active high | Contiki-NG `cc2538dk` dev/board.h + leds-arch.c; RIOT `cc2538dk` board.h |
| Keys | SELECT PA3, LEFT PC4, RIGHT PC5, UP PC6, DOWN PC7, active low, pull-up | Contiki-NG `cc2538dk` dev/board.h + board-buttons.c |
| Console UART | UART0 PA0 (RX) / PA1 (TX), 115200 8N1 | Contiki-NG dev/board.h; RIOT periph_conf.h |

## Explicitly not claimed

- **SYS_CTRL clock gating** has no effect: the gates are stored and read back,
  but no peripheral is gated by them. Oscillator start-up timing, power modes
  and software reset are not modelled. `SYNC_32K` is a 32 768-Hz square wave
  from the cycle count.
- **IOC** pin muxing and pad overrides are a stub. The firmware writes them as
  §18.5 requires, but the console model does not depend on them.
- **GPIO interrupts** (`GPIO_RIS`/`GPIO_MIS`) are never raised; the AFSEL commit
  lock is not enforced.
- **RF core:** the radio is alone in an empty channel. Nothing is ever
  received, a sent frame goes nowhere, no RF interrupt is raised, and frame
  filtering, the CSP program, the MAC timer count and the analog front end are
  not modelled.
- **ROM:** no serial boot loader (an invalid CCA parks in WFI), no backdoor
  pin, no RAM BIST. `PageErase`/`ProgramFlash` drive the flash controller;
  `GetChipId`/`GetFlashSize` read `DIECFG0`.
- **Sleep timer:** the watchdog never bites, and capture is not modelled. The
  32-kHz rate assumes the CPU runs at `clock_hz` (16 MHz).
- **Cryptoprocessor:** the slave-interface (non-DMA) data path, `KEY2`/`KEY3`,
  pulse interrupts and processing latency are not modelled.
- **PKA, SSI0/1, I2C, GPTimer0-3, USB, uDMA, SoC ADC** are declared stubs.

See `VALIDATION.md` for the tier table.
