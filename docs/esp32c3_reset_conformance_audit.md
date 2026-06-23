# ESP32-C3 — Validation Audit Trail

Onboarding of the ESP32-C3 peripheral blocks that were present as declarative
descriptors under `configs/peripherals/esp32c3/` but **not wired** into
`configs/chips/esp32c3.yaml`. Validated against real silicon per
[`docs/peripherals.md`](peripherals.md).

## Hardware

| | |
|---|---|
| Board | ESP32-C3 (QFN32), revision **v0.4** |
| MAC | `38:44:be:42:f5:58` |
| Transport | built-in USB-Serial/JTAG (`303a:1001`) |
| Capture tool | `openocd-esp32 v0.12.0-esp32-20260424`, `board/esp32c3-builtin.cfg` |
| Identification | `esptool v5.3.0 chip-id` |

## What changed

`configs/chips/esp32c3.yaml` originally wired only `uart0`, `gpio`, `timg0`,
`interrupt_core0`, `rom` (5 blocks). The unwired blocks caused reads/writes in
their MMIO windows to fault or RAZ, which stalls real C3 firmware. The chip is
now onboarded to its **full SVD-documented estate** in two passes:

**Pass 1 — control blocks:** `uart1`, `timg1`, `system`, `rtc_cntl`, `apb_ctrl`,
`systimer`, `io_mux`, `i2c0`, `spi2`, `ledc`, `rmt`.

**Pass 2 — estate completion:** `spi0`, `spi1`, `gpio_sd`, `efuse`, `uhci0`,
`uhci1`, `bb`, `twai0`, `i2s0`, `aes`, `sha`, `rsa`, `ds`, `hmac`, `dma` (GDMA),
`apb_saradc`, `usb_device` (USB-Serial/JTAG), `sensitive`, `extmem`, `xts_aes`,
`assist_debug`.

Bases are authoritative from the ESP32-C3 SVD
(`tests/fixtures/real_world/esp32c3.svd`). Hardware RNG is intentionally *not* a
separate window: `RNG_DATA` lives at `APB_CTRL+0xB0` and is already modeled as
the `apb_ctrl` `RND_DATA` register.

## Silicon capture

Two reset-state captures are committed as oracles under
`scripts/hw-oracle/captures/esp32c3/<utc-ts>/reg_oracle.json`:

- **`20260611T161223Z`** — 15 control-block windows (592 words), pass 1.
- **`estate-20260611T193134Z`** — 21 estate windows, pass 2. `mdw` reads are
  wrapped in tcl `capture {}` so the data lands in `openocd.log` reliably in
  `-c` batch mode.

## Oracle diff result

**Pass 1:** of **423** registers that overlapped a descriptor `reset_value`,
**366 matched silicon (86.5%)**.

**Pass 2:** **94 non-zero** descriptor reset values matched the live C3 exactly
across the estate blocks — SPI0/1 config (`CTRL`/`CLOCK`/`USER*`), the full
SAR-ADC config, the entire SENSITIVE PMS permission estate (28 regs), EXTMEM
cache config + flash/PSRAM virtual-address windows (`0x42000000..0x427fffff`,
`0x3c000000..0x3c7fffff`), GPIO sigma-delta defaults, USB-Serial/JTAG
`CONF0`/`MEM_CONF`/`DATE`, and the XTS-AES date stamp. The crypto/DMA
accelerators (AES, SHA, RSA, DS, HMAC, GDMA, I2S0) read **all-zero idle**,
matching their descriptors and proving the windows map without bus fault. The
per-block divergences are the expected chip-specific/dynamic registers: `efuse`
holds this die's burned values (MAC/calibration), `twai0` reads its
reset-mode state, and USB FIFO/status registers are live.

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
**ROM-untouched, static** subset where descriptor and silicon agree — **75
representative registers** across the full wired estate (28 from pass 1, 47 from
pass 2).

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

> **Scope of this audit.** This is the *reset-state* onboarding trail: it
> validated the documented SVD estate maps cleanly and matches silicon at reset.
> Items listed below as "follow-ups" at the time of writing have since landed.

## Since this audit

- **WiFi/BT MAC** register modeling — **done**. The MAC windows were captured
  from a running radio (the map is not in the SVD) and modeled at register level,
  retiring the thunk-backed path. See
  [`docs/esp32c3_wifi_mac_bridge.md`](esp32c3_wifi_mac_bridge.md).
- Behavioral (running-firmware PC-trace) oracle for C3 — this audit covered
  reset-state fidelity + clean mapping for the full documented estate; the
  running-firmware oracle is exercised by the WiFi/lwIP bring-up above.
