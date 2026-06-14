# ESP32-C3

The Espressif ESP32-C3 (single-core RISC-V RV32IMC, 400 KB SRAM, external
QSPI flash, Wi-Fi + BLE) is LabWired's reference RISC-V Espressif target.
All declared peripherals are **declarative** — the chip yaml references
external YAML descriptors under `configs/peripherals/esp32c3/`.

## Status at a glance

| Aspect              | Status                                                                            |
|---------------------|-----------------------------------------------------------------------------------|
| Chip yaml           | [`configs/chips/esp32c3.yaml`](../../configs/chips/esp32c3.yaml)                  |
| System yaml         | [`configs/systems/esp32c3-devkit.yaml`](../../configs/systems/esp32c3-devkit.yaml) |
| Reference firmware  | [`crates/firmware-esp32c3-demo/`](../../crates/firmware-esp32c3-demo/) (RISC-V `riscv32imc` demo) |
| Validation          | reset-state conformance vs silicon — see [`docs/esp32c3_reset_conformance_audit.md`](../esp32c3_reset_conformance_audit.md) |
| WiFi/lwIP           | full association → DHCP → UDP over the register-level MAC — see [`docs/esp32c3_wifi_mac_bridge.md`](../esp32c3_wifi_mac_bridge.md) |
| Tier                | full documented SVD estate wired + reset-state validated                          |

## Peripherals (from chip yaml)

The chip yaml now wires the **full documented SVD estate** — ~35 peripheral
descriptors. Behaviorally modeled blocks (RV32IMC core, UART0/1, GPIO, the WiFi
MAC + radio front-end) drive real firmware; the remainder are declarative
register windows validated for reset-state conformance and clean mapping (no
bus faults). Wired blocks include:

- **Core / console:** RV32IMC core, UART0/1, GPIO + `io_mux`, `gpio_sd`
- **Timers / clocks:** TIMG0/1, `systimer`, `system`, `apb_ctrl`, `rtc_cntl`
- **Buses:** SPI0/1/2, I²C0, I²S0, `rmt`, `ledc`, `twai0`, `uhci0/1`, GDMA (`dma`)
- **Radio:** `wifi_mac`, `radio_fe`, `radio_nrx`, `bb` (register-level MAC; see WiFi link above)
- **Crypto / security:** AES, SHA, RSA, HMAC, DS, XTS-AES, `sensitive`, `assist_debug`
- **Misc:** `efuse`, `apb_saradc`, `usb_device` (USB-Serial/JTAG), `extmem`

Authoritative bases come from the ESP32-C3 SVD
(`tests/fixtures/real_world/esp32c3.svd`); see
[`docs/esp32c3_reset_conformance_audit.md`](../esp32c3_reset_conformance_audit.md)
for the silicon oracle diff and which registers are static vs ROM-configured.
See [`docs/declarative_registers.md`](../declarative_registers.md) for how
declarative peripheral descriptors work.
