# Seeed XIAO nRF52840 Sense Validation

Validation date: 2026-05-27

## Hardware Presence

The connected board enumerated on USB as:

```text
2886:8045 Seeed Technology Co., Ltd. Seeed XIAO nRF52840 Sense
/dev/serial/by-id/usb-Arduino_Seeed_XIAO_nRF52840_Sense_329707A450F0C9C3-if00 -> ../../ttyACM5
```

USB descriptor evidence:

- Application VID:PID: `2886:8045`
- Manufacturer: `Arduino`
- Product: `Seeed XIAO nRF52840 Sense`
- Serial: `329707A450F0C9C3`
- Driver: `cdc_acm`
- Interfaces: CDC ACM control + CDC data

CDC serial reads at 115200, 9600, and 1200 baud produced zero bytes over a
3-second capture window.

## Bootloader Presence

A 1200-baud CDC touch switched the board into bootloader mode without writing
firmware:

```text
2886:0045 Seeed Technology Co., Ltd. XIAO nRF52840 Sense
/dev/serial/by-id/usb-Seeed_XIAO_nRF52840_Sense_940D8A73707DC298-if00 -> ../../ttyACM5
```

Bootloader descriptor evidence:

- Bootloader VID:PID: `2886:0045`
- Manufacturer: `Seeed`
- Product: `XIAO nRF52840 Sense`
- Serial: `940D8A73707DC298`
- Driver: `cdc_acm`
- Interfaces: CDC ACM control + CDC data

No UF2 mass-storage volume appeared under `/media`, `/run/media`, or `/mnt`.
`dfu-util -l` did not list a DFU interface. A read-only Nordic serial DFU ping
sent over CDC at 9600, 57600, 115200, 230400, and 1000000 baud returned no
bytes. No firmware was written to the device during validation.

No external SWD debug probe was visible through `probe-rs list` or
`nrfjprog --ids`, so this validation does not claim SWD register dumps,
flash programming, or logic-level SPI waveform parity.

## Simulator Evidence

Commands:

```bash
cargo test -p labwired-core nrf52::xiao -- --nocapture
cargo build -p firmware-nrf52840-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/seeed-xiao-nrf52840-sense/uart-gpio-spi-smoke.yaml \
  --output-dir out/seeed-xiao-nrf52840-sense/uart-gpio-spi-smoke \
  --no-uart-stdout
```

Expected artifacts:

- `out/seeed-xiao-nrf52840-sense/uart-gpio-spi-smoke/result.json`
- `out/seeed-xiao-nrf52840-sense/uart-gpio-spi-smoke/uart.log`
- `out/seeed-xiao-nrf52840-sense/uart-gpio-spi-smoke/junit.xml`

The UART artifact must contain `NRF52840_SMOKE_OK`.

## Coverage

| Area | Evidence |
|------|----------|
| Board manifest | `xiao_nrf52840_sense_manifest_builds_with_uart_gpio_spi` |
| GPIO | `xiao_nrf52840_gpio_task_registers_drive_led_pins` |
| SPI | `xiao_nrf52840_spim0_start_sets_end_event_and_amount` |
| UART | `uart-gpio-spi-smoke.yaml` assertion for `NRF52840_SMOKE_OK` |

## Known Limits

- GPIO1 uses a synthetic non-overlapping LabWired base address
  `0x50001000`. Nordic maps P1 registers inside the same GPIO block region as
  P0; LabWired's current bus model expects non-overlapping peripheral ranges.
- SPIM0 implements register/task smoke behavior only. EasyDMA memory movement
  and SPI device byte routing are future work.
- GPIO1 uses a synthetic non-overlapping LabWired base address (see above);
  this is a model convenience, not a silicon divergence.

## Silicon validation over SWD (2026-06-03)

The board was connected to an ST-Link/V2 (SWDIO/SWCLK/GND/3V3) and both the
hardware-oracle diff banks were run against the physical chip — **all green**:

| Harness | Cases | Result |
|---------|-------|--------|
| `nrf52_mmio_diff` (GPIO0 OUT/DIR, UART0 ENABLE, SPIM0 PSEL/FREQ/MAXCNT, …) | 16 | match=16 diverge=0 |
| `nrf52_onboarding_diff` (FICR, RADIO, USBD, SAADC, RNG, TIMER0, RTC0, PWM0, QSPI, NFCT, NVMC, GPIOTE, PPI, ECB, COMP, TEMP, WDT, QDEC, EGU0, PDM, ACL, CRYPTOCELL) | 30 | match=30 diverge=0 |

Identity read straight off the silicon: CPUID `0x410FC241` (Cortex-M4 r0p1),
FICR `INFO.PART = 0x52840`, RAM 256 KB, FLASH 1 MB, APPROTECT unlocked — all
match the simulator's FICR model. (FICR `INFO.VARIANT` is chip-batch specific —
this unit reads `AAD0` — so it is intentionally not asserted.)

**Firmware execution on silicon (flash + run).** `firmware-nrf52840-demo` was
flashed to the chip over SWD (`program … verify reset` — OpenOCD reported
`nRF52840-xxAA(build code: D0)`, matching FICR VARIANT `AAD0`) and run. Halting
and reading back over SWD confirms it executed the same register writes the
simulator models for the same ELF:

| Register | Silicon | Simulator |
|----------|---------|-----------|
| UART0.ENABLE `0x40002500` | `0x4` | `0x4` |
| SPIM0.TXD.MAXCNT `0x40003548` | `0x4` | `0x4` |
| SPIM0.TXD.PTR `0x40003544` | →`SPI_SMOKE_BYTES` | →`SPI_SMOKE_BYTES` |
| UART0.TXD stream | `NRF52840_SMOKE_OK` (to pin) | `NRF52840_SMOKE_OK` (UART sink) |

(The plain ST-Link/V2 has no VCP and the demo's UART pin isn't wired out, so the
TXD byte stream is captured on the sim side and proven on silicon via the
register state + the running loop. Flashing overwrote the XIAO UF2 bootloader,
as expected for a bare-metal SWD flash; re-flash the Seeed bootloader to restore
USB-UF2.)

Reproduce (works with multiple ST-Links attached — select the nRF probe by
serial via the env var added to the OpenOCD helper):

```bash
LABWIRED_STLINK_SERIAL=<nrf-stlink-serial> \
  cargo test -p labwired-hw-oracle --test nrf52_mmio_diff \
    --features hw-oracle-nrf52 -- --ignored --nocapture
LABWIRED_STLINK_SERIAL=<nrf-stlink-serial> \
  cargo test -p labwired-hw-oracle --test nrf52_onboarding_diff \
    --features hw-oracle-nrf52 -- --ignored --nocapture
```

### Regression protection

- **CI (no hardware):** `test_nrf52840_demo_survival` (asserts `NRF52840_SMOKE_OK`)
  and the `seeed-xiao-nrf52840-sense` strict-onboarding `[PASS]`. The survival
  test had been crashing on a latent DMA-model underflow (DMA `ISR`/`IFCR`
  read at offset `0x00`/`0x04` did `offset - 0x08`); fixed in
  `crates/core/src/peripherals/dma.rs`.
- **Manual silicon re-check:** the two hardware diff banks above.
