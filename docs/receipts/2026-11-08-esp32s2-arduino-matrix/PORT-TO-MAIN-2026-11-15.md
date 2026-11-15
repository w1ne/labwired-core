# ESP32-S2 Arduino row: port from the lane to `main` (2026-11-15)

The receipts in this directory were measured on `lane/new-hw-onboarding`
on 2026-11-08. This note records what changed when the row moved to `main`.

## What changed in the port

- **RMT (L2).** `main` has its own classic-ESP32 RMT model (#1692, #1813),
  which differs from the lane's model. The S2 RMT is now a separate file,
  `crates/core/src/peripherals/esp32/rmt_s2.rs` (type `esp32s2_rmt`). The
  classic RMT on `main` is not changed.
- **CLI.** `main` builds the S2 through
  `build_esp32s2_system_from_manifest_with_plugins`. When a
  `<stem>.partitions.bin` sits beside the ELF, `labwired test` now attaches
  the S2 external flash and runs `boot::esp32s2::fast_boot`. The freestanding
  DevKitC-1-N8R2 smoke (no partition table) keeps its direct-ELF path.
- **Layout.** Integration tests are modules of the `esp` test binary
  (`crates/core/tests/esp/esp32s2_arduino_*.rs`). The matrix board is
  `validation/arduino-matrix/boards/esp32s2.yaml`. Bus proof is
  `validation/bus_proof/esp32s2-arduino.json`; SPI and I2C are now `proven`
  (the lane file still said `none`, which was stale after L3/L4 landed).
- **Not ported.** The lane's DevKitM-1-N4R2 GPIO smoke profile is not part of
  this change.

## Evidence on `main`

`cargo test -p labwired-core --test esp esp32s2` on the port branch:
11 passed, 0 failed. Positive cells: L0 boot, L2 RMT WS2812 on GPIO18,
L3 I2C INA219, L4 SPI MAX31855, L5 ADC fixed mid-scale, L6 LEDC 1 kHz on
GPIO2, L8 TWAI self-RX. Controls: L5 ADC follows the applied voltage on
ADC1_CH0; erased partition table never reaches `setup()`; no INA219 gives a
NACK; no MAX31855 reads 0xffffffff.

## Arduino matrix row on `main`

`run_matrix.py --boards esp32s2` with the port branch's CLI, all nine stock
sketches compiled fresh by PlatformIO `espressif32` 8.0.0
([log](main-port-matrix-esp32s2-2026-11-15.log),
[scoreboard](main-port-scoreboard-2026-11-15.md),
[results, gzip](main-port-results-2026-11-15.json.gz)):

**9 pass / 0 skip / 0 fail.**

| cell | pad / oracle measurement |
|---|---|
| L2 blink | 96 edges on the GPIO18 pad (WS2812 frames), 2 RMT TX_STARTs; oracle needs ≥ 48 edges and ≥ 1 TX_START |
| L6 PWM | GPIO2 pad (`LEDC_LS_CH6`): period 240,000 cycles = 1.000 kHz at 240 MHz, high 120,000 cycles = 50.0 % duty, 58 edges |

The other cells pass on their UART markers, which the sketches print only
after exact value checks (L3 INA219 registers, L4 MAX31855 frame, L5 code
2048, L8 TWAI ID 0x123 / data 0xA5).
