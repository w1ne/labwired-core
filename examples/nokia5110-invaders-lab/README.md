# Nokia 5110 "Invaders" Lab

A small interactive demo on **STM32L476** (Cortex-M4F): a Nokia-5110 / PCD8544
84×48 monochrome LCD over SPI1, with player position driven by an **HC-SR04**
ultrasonic distance sensor (GPIO-timed echo). Wave your hand closer/further and
the on-screen ship tracks the distance — a tiny Space-Invaders-style loop.

This lab combines a **display + a sensor of a different bus class** (SPI LCD +
GPIO-timed ranger), so its real output is the **LCD framebuffer**, not UART —
view it in the web playground. The `system.yaml` sets `walk_deleted: true` (the
firmware drives only SPI1 + GPIO; the HC-SR04 echo is serviced outside the walk),
which roughly doubles simulation throughput in the browser and is verified
byte-identical to the walk-free path.

Run from the repo root (note the Cortex-M4F target):

```bash
cargo build -p nokia5110-invaders-lab --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nokia5110-invaders-lab/io-smoke.yaml
```

The CLI smoke (`io-smoke.yaml`) is a **clean-run** check: it boots the firmware,
attaches the PCD8544 LCD and HC-SR04 ranger, and runs the game loop for 5M steps
with no faults or unmapped accesses. The pixels are surfaced to the playground's
display overlay for the visual experience.

Devices (in `system.yaml`): `pcd8544` LCD on SPI1 (CS `PB6`, D/C `PC7`) and
`hc-sr04` ranger (TRIG `PA8`, ECHO `PB10`).
