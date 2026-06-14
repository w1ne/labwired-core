# ILI9341 TFT Lab

This firmware drives an ILI9341 240×320 colour TFT over LabWired's simulated
SPI1 path on STM32F103: it runs the ILI9341 init sequence, then draws colour
bars and solid red/green/blue bands, flushing a full frame.

Run from the repo root:

```bash
cargo build -p ili9341-tft-lab --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- test --script examples/ili9341-tft-lab/io-smoke.yaml
```

Expected UART:

```text
ILI9341 TFT Lab
TFT init done
colour bars drawn
...
frame done
```

The panel attaches over SPI1 with chip-select on `PA4` (`device_type: ili9341`
in `system.yaml`). The rendered frame is best viewed in the web playground.
