# ESP32-S3 Doom-like community lab

This is a small, original one-level ray-cast game for an ESP32-S3, an ILI9341
LCD, and six ordinary active-low push buttons. It is Doom-inspired, but it does
not contain Doom code, maps, graphics, sounds, or other id Software assets.

## Play in LabWired

Open `?board=esp32s3-doomlike-lab`, click **Run**, and wait for
`DOOMLIKE_READY` in Serial. The controls are the six buttons on the circuit:

- Forward / Back: walk
- Left / Right: turn
- Fire: shoot the nearest visible enemy
- Use: open the door or activate the exit tile

Reach the exit tile and press **Use** to win. **Reset** restarts the firmware and
level. Everything visible and interactive is part of the normal embedded
circuit; there is no game-specific browser overlay or keyboard handler.

## Rendering and memory

Gameplay renders with integer-only math into a static 160×120 RGB565 buffer
(38,400 bytes). Each source row is expanded into one 640-byte scanline and sent
twice, producing the ILI9341's 320×240 landscape image without allocating a
second framebuffer. The release ELF is 2,357,216 bytes; the target board used
for validation reports 4 MiB flash and 2 MiB PSRAM.

All game art and palettes in `src/assets.rs` were created for this demo and are
licensed under the repository's MIT license.

## Build and verify

From this directory:

```sh
cargo +stable test --lib --target aarch64-apple-darwin
cargo build --release --features hw
```

The built firmware is
`target/xtensa-esp32s3-none-elf/release/esp32s3-doomlike-lab`. The deterministic
initial render hashes to FNV-1a `0x83a7228e`.

The current physical-board proof covers the ESP32-S3 itself (4 MiB flash,
2 MiB PSRAM). Physical LCD and touch validation is deliberately still open:
the exact LCD/touch controller on the available module must be identified
before its pinout or touch protocol can be claimed. The browser community demo
uses the modeled ILI9341 and push-button circuit described in `system.yaml`.
