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
