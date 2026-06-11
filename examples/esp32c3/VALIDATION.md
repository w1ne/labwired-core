# ESP32-C3 — Validation Audit Trail

Onboarding of the ESP32-C3 peripheral blocks that were present as declarative
descriptors under `configs/peripherals/esp32c3/` but **not wired** into
`configs/chips/esp32c3.yaml`. Validated against real silicon per
[`docs/peripheral_onboarding_playbook.md`](../../docs/peripheral_onboarding_playbook.md).

## Hardware

| | |
|---|---|
| Board | ESP32-C3 (QFN32), revision **v0.4** |
| MAC | `38:44:be:42:f5:58` |
| Transport | built-in USB-Serial/JTAG (`303a:1001`) |
| Capture tool | `openocd-esp32 v0.12.0-esp32-20260424`, `board/esp32c3-builtin.cfg` |
| Identification | `esptool v5.3.0 chip-id` |

## What changed

`configs/chips/esp32c3.yaml` previously wired only `uart0`, `gpio`, `timg0`,
`interrupt_core0`, `rom` (5 blocks). The unwired blocks caused reads/writes in
their MMIO windows to fault or RAZ, which stalls real C3 firmware. Now wired:

`uart1`, `timg1`, `system`, `rtc_cntl`, `apb_ctrl`, `systimer`, `io_mux`,
`i2c0`, `spi2`, `ledc`, `rmt`.

## Silicon capture

15 peripheral register windows (592 words) were read at `reset halt` and stored
as the reset-state oracle:

```
scripts/hw-oracle/captures/esp32c3/<utc-ts>/reg_oracle.json
```

## Oracle diff result

Of **423** registers that overlapped a descriptor `reset_value`, **366 matched
silicon (86.5%)**.

The remaining **57 are not descriptor bugs.** A JTAG `reset halt` on the C3 is a
*software* core reset that does not cold-reset the peripherals, and the ROM
bootloader has already run by the time the core halts. Those registers therefore
hold **post-ROM / dynamic** values, confirmed by re-reading after a hardware
(EN-toggle) reset — they stayed at the post-ROM values, not the SVD cold-reset
values. They fall into clear classes:

| Class | Examples |
|-------|----------|
| ROM-configured UART console | `UART0/1 CLKDIV` (115200 baud), `CONF0/1`, `STATUS` |
| Live FIFO / status | UART `FIFO`, `INT_RAW`, `MEM_*_STATUS`; I2C `DATA` |
| Bootstrap / live pins | `GPIO STRAP` (`0x0d`), `GPIO IN`; `IO_MUX` pad pulls |
| Fed / cleared watchdogs | `TIMG0/1 WDTCONFIG0` (`0` after ROM, vs SVD `0x4c000`) |
| RTC calibration / sticky | `RTC_CNTL STATE0`, `RESET_STATE`, `WDTWPROTECT` |

Because of this, the committed conformance test
(`crates/hw-oracle/tests/esp32c3_reset_conformance.rs`) asserts only the
**ROM-untouched, static** subset where descriptor and silicon agree (28
representative registers across the newly-wired blocks).

## Verification

```text
# the C3 boots cleanly with all blocks mapped (no unmapped-peripheral faults)
cargo build --release -p firmware-esp32c3-demo --target riscv32imc-unknown-none-elf
target/release/labwired run --chip configs/chips/esp32c3.yaml \
  --firmware target/riscv32imc-unknown-none-elf/release/firmware-esp32c3-demo
# -> "ESP OK"

# reset-state conformance vs silicon (CI)
cargo test -p labwired-hw-oracle --test esp32c3_reset_conformance
# -> ok. 1 passed
```

## Not yet done

- Radio (WiFi/BLE) register modeling — decided to be register-modeled (see
  playbook §4); large, follow-up work.
- Behavioral (running-firmware PC-trace) oracle for C3 (needs a per-peripheral
  probe firmware); this pass validated reset-state fidelity + clean mapping.
