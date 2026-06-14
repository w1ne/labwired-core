# SSD1306 Hello Lab

This firmware drives a 128×64 SSD1306 OLED over LabWired's simulated I²C1 path
on STM32F103: it runs the SSD1306 init sequence, writes a frame into the paged
framebuffer, and flushes it to the panel. The display model tracks the
8-page × 128-column framebuffer; the WASM bridge surfaces pixel state for the
playground's display overlay.

Run from the repo root:

```bash
cargo build -p ssd1306-hello-lab --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- test --script examples/ssd1306-hello-lab/io-smoke.yaml
```

Expected UART:

```text
SSD1306 Hello Lab
OLED init done
OLED render done
```

The OLED attaches over I²C1 at address `0x3C` (`device_type: oled-ssd1306` in
`system.yaml`). The visual framebuffer is best viewed in the web playground.
