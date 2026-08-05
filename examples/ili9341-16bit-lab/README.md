# ILI9341 16-bit parallel lab (MRB3205-class)

Smoke path for the **LCDWiki MRB3205** electrical contract: 3.2″ ILI9341 over
**16-bit 8080 parallel** (`ili9341-16bit`), driven by **classic ESP32 GPIO**.

## Why classic ESP32 (this lab)

This lab’s pin map and firmware are classic-ESP32-shaped. Edge delivery itself
is multi-family: `SystemBus::install_gpio_observer` also wires ESP32-S3,
ESP32-C3, and STM32/nRF `GpioPort` banks (STM32 uses global pin ids
`PAx = x`, `PBx = 16+x`, …). See `e2e_gpio_edge_all_families` for C3 + STM32
paint proofs. STM32 FMC (parallel memory controller) remains out of scope.

## Pin map

| Role | GPIO | Role | GPIO |
|------|------|------|------|
| CS   | 15   | DB0  | 12   |
| RS   | 2    | DB1  | 13   |
| WR   | 4    | DB2  | 14   |
| RD   | 5    | DB3  | 16   |
| RST  | 33   | DB4  | 17   |
|      |      | DB5  | 18   |
|      |      | DB6  | 19   |
|      |      | DB7  | 21   |
|      |      | DB8  | 22   |
|      |      | DB9  | 23   |
|      |      | DB10 | 25   |
|      |      | DB11 | 26   |
|      |      | DB12 | 27   |
|      |      | DB13 | 32   |
|      |      | DB14 | 0    |
|      |      | DB15 | 3    |

Matches `system.yaml` and the e2e smoke test.

## Prove it on the twin

```bash
cd core
cargo test -p labwired-core --test e2e_ili9341_16bit_gpio_smoke -- --nocapture
```

The test:

1. Loads this `system.yaml` through `SystemBus::from_config` (same attach path as compile emit).
2. Bit-bangs 8080 via real ESP32 GPIO OUT/OUT1 registers (W1TS/W1TC) — the path
   firmware would take with `digitalWrite` / register pokes.
3. Asserts the panel has non-zero ink and inspect reports a painted framebuffer.

## Catalog / playground

- Catalog type: `ili9341-16bit`
- Manufacturer part: `lcdwiki-mrb3205` (34-pin silkscreen, touch unmodelled)
- Diagram compile emits `connection: "gpio"` + the pin map above when wired

## Touch / SD

Not part of this smoke. MRB3205 T_* / SDCS pads are documented as unmodelled.
