# ESP32 E-Paper Lab

ESP32-WROOM-32 driving a Waveshare 2.9" tri-color e-paper (SSD1680 / GDEM029C90)
over VSPI. Rust + `esp-hal`; same ELF runs on real silicon (via `espflash`) and
inside the LabWired simulator.

## What it does
Draws three full-width horizontal bands — WHITE / BLACK / RED — and triggers
one full panel refresh. The byte sequence on the wire mirrors what the
AgentDeck firmware (`GxEPD2_290_C90c`) and the LabWired STM32 e-paper lab emit,
so the simulator's SSD1680 model decodes all three paths identically.

## Pin mapping (Waveshare default, AgentDeck-compatible)

| Signal | ESP32 GPIO | Notes                       |
|--------|------------|-----------------------------|
| CS     | GPIO5      | GPIO output push-pull       |
| SCK    | GPIO18     | VSPI signal, IO_MUX func 1  |
| MOSI   | GPIO23     | VSPI signal, IO_MUX func 1  |
| DC     | GPIO17     | GPIO output push-pull       |
| RST    | GPIO16     | GPIO output push-pull       |
| BUSY   | GPIO4      | GPIO input                  |

## Toolchain
This crate targets `xtensa-esp32-none-elf` and **must build out of the main
workspace** (excluded in the root `Cargo.toml`). It requires the `esp` Rust
toolchain:

```bash
# one-time toolchain install
cargo install espup
espup install
. "$HOME/export-esp.sh"   # or whatever espup printed
```

## Build

```bash
cd examples/esp32-epaper-lab
cargo build --release
```

The ELF lands at `target/xtensa-esp32-none-elf/release/esp32-epaper-lab`.

## Run in the simulator

```bash
labwired run \
  --system examples/esp32-epaper-lab/system.yaml \
  --firmware examples/esp32-epaper-lab/target/xtensa-esp32-none-elf/release/esp32-epaper-lab
```

The simulator wires the SSD1680 to the modeled VSPI controller per
`system.yaml`. Captured SPI byte stream is byte-for-byte compatible with the
AgentDeck path and the STM32 epaper-tricolor-lab.

## Flash to real hardware

```bash
espflash flash \
  --port /dev/ttyUSB0 --baud 460800 --monitor \
  target/xtensa-esp32-none-elf/release/esp32-epaper-lab
```

## Status (v0.15.0)
- Firmware builds cleanly for `xtensa-esp32-none-elf`.
- Real ESP32-WROOM-32 hardware: ✅ paints the three bands and refreshes the
  panel.
- LabWired sim: partial — esp-hal's `Reset → __pre_init → esp32_init` chain
  touches DPORT / IO_MUX / RTC banks that the v0.15.0 sim doesn't yet model
  with enough fidelity to dispatch into `main`. The Arduino-ESP32 path (see
  `examples/labwired-ereader-arduino/`) gets all the way to a painted SSD1680
  via the ROM-thunk pipeline shipped in v0.15.0 — this lab is the equivalent
  pure-Rust path, blocked on `__pre_init` coverage. Tracked as a follow-up.

## See also
- [`examples/labwired-ereader-arduino/`](../labwired-ereader-arduino/) —
  same panel, Arduino-ESP32, sim-paints in v0.15.0 via the runtime-snapshot
  blob and ROM thunks.
- [`examples/epaper-tricolor-lab/`](../epaper-tricolor-lab/) — STM32F103
  variant, byte-for-byte compatible SSD1680 driver.
