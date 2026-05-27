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
- Hardware validation is USB enumeration, bootloader entry, and serial
  availability only until SWD or external measurement hardware is attached.
