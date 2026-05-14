[← Back to Hub](../README.md)

# Wokwi Project Import — Feasibility & Mapping

**Date:** 2026-05-14
**Status:** Research / feasibility. Implementation is a Phase 2 deliverable, scoped after the [Playground Device Library Phase 1](../superpowers/plans/2026-05-14-playground-device-library-phase1.md) closes the bus matrix.

## Why this exists

Wokwi is the category-defining hardware sandbox. Most public Arduino / ESP32 / STM32 / Pico examples on GitHub, in blog posts, and pasted into agent prompts are already authored against Wokwi's `diagram.json`. Every project we can import without manual rewiring is a project that lands on LabWired instead of bouncing.

The strategic claim: **if a user can paste a Wokwi project URL and have it open in LabWired, then LabWired's silicon-parity + CI + agent story becomes a *strict upgrade* path for the Wokwi user base** — not a competing ecosystem they have to learn from scratch.

This document maps Wokwi's two project formats onto LabWired's `system.yaml` + canvas state, identifies the importable subset, and flags the blockers.

---

## What we are mapping

### Wokwi side

| File | Role |
|---|---|
| [`diagram.json`](https://docs.wokwi.com/diagram-format) | The canvas: parts, positions, attrs, connections (wires) |
| [`wokwi.toml`](https://docs.wokwi.com/vscode/project-config) | Project config: firmware path, optional ELF, custom chip bindings |
| `<chip>.chip.wasm` + descriptor | User-authored custom chip (out of scope for import) |
| `wokwi-cli` scenarios | YAML test runner (separate import target — closer to `labwired test`) |

### LabWired side

| File | Role |
|---|---|
| `core/configs/chips/<chip>.yaml` | Chip descriptor (memory map, peripherals) |
| `core/configs/systems/<system>.yaml` (or `examples/<lab>/system.yaml`) | `SystemManifest` — chip ref + `external_devices` + `board_io` |
| Editor diagram (in-memory) | `Diagram` shape from `packages/ui/src/editor/types.ts` — parts, wires |
| Built firmware ELF in `packages/playground/public/wasm/` | Pre-built artifact loaded by playground |

---

## Concept mapping

| Wokwi concept | LabWired concept | Notes |
|---|---|---|
| `diagram.json` `parts[]` | Editor `Diagram.parts[]` + `system.yaml` `external_devices[]` | One Wokwi part becomes one editor part *and* one external_devices entry (for protocol devices) |
| Part `id` | Editor `part.id` + `external_devices[].id` + `board_io[].id` | Pass through verbatim |
| Part `type` (e.g. `wokwi-ssd1306`) | Editor `ComponentDef.type` + bus device type | Strip `wokwi-` prefix; many already align (`led`, `button`, `ssd1306`, `dht22`) |
| Part `attrs` | Editor `part.attrs` + `external_devices[].config` | Most attrs map 1:1 (e.g. `color`, `i2c_address`); some are visual-only (e.g. `gamma`, `fps`) |
| Part `left` / `top` | Editor `part.x` / `part.y` | Pass through. Wokwi origin is also top-left. |
| Part `rotate` | Editor `part.rotate` | Pass through |
| `connections[]` (`["a:pin", "b:pin", color, []]`) | Editor `Diagram.wires[]` | Trivial transform. Drop the placement-instructions array — our wire router regenerates it. |
| Wokwi board part (e.g. `wokwi-uno`, `wokwi-esp32-devkit-c-v4`) | LabWired chip+system YAML pair | **The hard mapping** — see board map below |
| `wokwi.toml` `firmware` | `BoardConfig.demoFirmwarePath` | If user uploads ELF, drop into `public/wasm/`; otherwise reject with "compile-in-browser is Phase 2" |
| `wokwi.toml` `[[chip]]` custom chip | (no equivalent — reject for now) | Phase 3 deliverable: WASM custom-device API |
| `wokwi.toml` `[[net.forward]]` | (no equivalent — drop) | LabWired has no WiFi gateway today |

---

## Board mapping (the hard part)

Wokwi's board parts encode the chip + pin layout + clock together. LabWired splits chip (memory+peripherals) from system (board IO + pin labels). The mapping is many-to-one — multiple Wokwi boards point at the same LabWired chip but different `system.yaml`.

| Wokwi board part | LabWired chip | LabWired system | Notes |
|---|---|---|---|
| `wokwi-uno` | (none — ATmega328P not yet supported) | — | **AVR is the biggest gap.** See "Blockers" below. |
| `wokwi-nano` | (none) | — | AVR |
| `wokwi-mega` | (none) | — | AVR |
| `wokwi-esp32-devkit-c-v4` | `esp32s3.yaml` (closest) or new `esp32.yaml` | `esp32s3-zero.yaml` (closest) | LabWired's ESP32 support is S3-flavored today; classic ESP32 (Xtensa LX6, dual-core) needs a separate descriptor |
| `wokwi-esp32-s3-devkitc-1` | `esp32s3.yaml` | `esp32s3-zero.yaml` | Closest existing pair. Pin labels need a layout shim. |
| `wokwi-esp32-c3-devkitm-1` | `esp32c3.yaml` | `esp32c3-devkit.yaml` | Direct match |
| `wokwi-pi-pico` | `rp2040.yaml` | `rp2040-pico.yaml` | Direct match |
| `wokwi-pi-pico-w` | `rp2040.yaml` | `rp2040-pico.yaml` (no WiFi) | WiFi drops silently |
| `wokwi-pi-pico-2` | (none — RP2350 not supported) | — | RP2350 future work |
| `wokwi-nucleo-f103rb` | `stm32f103.yaml` | new `nucleo-f103rb.yaml` | Need to author |
| `wokwi-stm32-blackpill-stm32f401` | `stm32f401cdu6.yaml` | `stm32f401cdu6-blackpill.yaml` | Direct match (recently added) |

**Decision:** Phase-2 import targets RP2040, ESP32-C3, ESP32-S3, STM32 Black Pill, and the two existing Nucleo systems. AVR (Uno/Nano/Mega) is explicitly deferred until LabWired ships an AVR core — that's its own multi-month track.

---

## Component (part) mapping

The Phase-1 device library covers most of what users actually wire up in Wokwi examples. Mapping table for the common parts:

| Wokwi part | LabWired editor type | LabWired external_devices type | Phase 1? |
|---|---|---|---|
| `wokwi-led` | `led` | (board_io GPIO) | Yes (already shipping) |
| `wokwi-pushbutton` | `button` | (board_io GPIO) | Yes |
| `wokwi-resistor` | `resistor` | (visual only) | Yes |
| `wokwi-buzzer` | `buzzer` | (board_io GPIO/PWM) | Yes |
| `wokwi-rgb-led` | `rgb-led` | (board_io GPIO ×3) | Yes |
| `wokwi-potentiometer` | `potentiometer` | analog_input | Yes (Wave 3) |
| `wokwi-photoresistor` | `ldr` | analog_input | Yes (Wave 3) |
| `wokwi-ntc-temperature-sensor` | `ntc-thermistor` (new) | analog_input | Yes (Wave 3) |
| `wokwi-dht22` | `dht22` | (1-Wire — **gap**) | No — 1-Wire deferred |
| `wokwi-ssd1306` | `ssd1306` (new) | `ssd1306` | Yes (Wave 1) |
| `wokwi-lcd1602` | `lcd1602` | `pcf8574-lcd1602` (new) | Maybe — needs PCF8574 |
| `wokwi-ili9341` | `ili9341` (new) | `ili9341` | Yes (Wave 2) |
| `wokwi-neopixel` / `wokwi-neopixel-ring` | `neopixel` | (board_io WS2812 — **gap**) | No — WS2812 timing-protocol deferred |
| `wokwi-servo` | `servo` | (board_io PWM) | Partial — needs PWM-decoder binding |
| `wokwi-hc-sr04` | `ultrasonic` | (echo-pin timing — **gap**) | No — needs trigger/echo timing modeling |
| `wokwi-ds1307` | (new) | `ds1307` | Phase 2 |
| `wokwi-mpu6050` | `mpu6050` (new) | `mpu6050` | Yes (Wave 1) |
| `wokwi-bmp280` / `wokwi-bme280` | `bme280` (new) | `bme280` | Yes (Wave 1) |
| `wokwi-adxl345` | `adxl345` (new) | `adxl345` | Yes (Wave 1) |
| `wokwi-sd-card` | (new) | `sd-card-spi` | Phase 2 |
| `wokwi-gps` (NEO-6M family) | `gps-neo6m` (new) | `gps-neo6m` | Yes (Wave 3) |
| `wokwi-7segment` | `seven-segment` | (board_io GPIO) | Yes |
| `wokwi-keypad` | `keypad` | (board_io GPIO matrix) | Yes |
| Custom `[[chip]]` (`*.chip.wasm`) | — | — | **Reject in v1.** Phase 3. |

**Coverage estimate.** Once Phase 1 ships, 70–80% of typical Wokwi public examples will import cleanly. NeoPixel, DHT22, HC-SR04, and AVR-Uno projects are the four largest remaining gaps.

---

## Translator design

```
┌─────────────────────────────────────────────────────────────┐
│                  Wokwi import pipeline                      │
└─────────────────────────────────────────────────────────────┘

  ┌─────────────┐       ┌──────────────────┐
  │  Wokwi URL  │─────▶ │  Fetch project   │  (wokwi.com/projects/<id> exposes
  │   pasted    │       │  via public API  │   diagram.json + sketch via API,
  └─────────────┘       └────────┬─────────┘   or user uploads diagram.json)
                                 │
                                 ▼
                  ┌──────────────────────────┐
                  │  Wokwi → LabWired        │
                  │  translator (TypeScript) │
                  │                          │
                  │  • Validate schema       │
                  │  • Map board part →      │
                  │    chip + system YAML    │
                  │  • Map parts →           │
                  │    editor + external_dev │
                  │  • Map connections →     │
                  │    wires                 │
                  │  • Collect warnings      │
                  └──────────┬───────────────┘
                             │
              ┌──────────────┴───────────────┐
              ▼                              ▼
   ┌──────────────────┐           ┌────────────────────┐
   │  Editor Diagram  │           │  system.yaml       │
   │  (canvas opens)  │           │  (used by core)    │
   └──────────────────┘           └────────────────────┘
                             │
                             ▼
                  ┌──────────────────────────┐
                  │  Import report           │
                  │  • parts mapped: N       │
                  │  • parts dropped: M      │
                  │  • blockers: [list]      │
                  │  • firmware needed       │
                  └──────────────────────────┘
```

### Surfaces

1. **`labwired import wokwi <url|path>`** — CLI command in `labwired-cli`. Writes `system.yaml` + `diagram.json` (LabWired's internal canvas state) + an `import-report.md`.
2. **Playground "Import from Wokwi" button** — UI button that accepts a paste of `diagram.json` (file or URL). Opens the imported board with all warnings shown in a side panel.
3. **`@labwired/wokwi-import` package** — TypeScript library housing the translator. Both surfaces above call into it. Lives at `packages/wokwi-import/`.

### Firmware handling

- If the user uploads a `.elf` / `.hex` / `.bin` alongside `diagram.json`: copy it to `public/wasm/`, set `demoFirmwarePath` accordingly.
- If only `diagram.json` is provided: import opens with no firmware loaded, banner reads *"Diagram imported. Upload a built firmware ELF to run."*
- `wokwi.toml` `firmware` references a path on the user's machine — for URL imports, that's not retrievable. Drop with a warning.

---

## Blockers & out-of-scope

| Blocker | Phase to address |
|---|---|
| **AVR architecture** (Uno / Nano / Mega) — no LabWired ATmega328P core today | Multi-month track, separate plan |
| **Custom `*.chip.wasm` chips** — Wokwi's WASM chip API has no LabWired equivalent | Phase 3 (model registry + WASM custom-device API) |
| **WS2812 / NeoPixel** — bit-banged timing protocol | Phase 2 (needs cycle-accurate GPIO observer or dedicated bus) |
| **1-Wire (DHT22, DS18B20)** — bit-banged timing protocol | Phase 2 |
| **HC-SR04 ultrasonic** — trigger/echo pulse-width timing | Phase 2 |
| **WiFi / BLE simulation** (`[[net.forward]]`, `wokwi-iot-mqtt`) | Out of scope. LabWired stays HW-level. |
| **Logic Analyzer VCD export differences** | LabWired already emits VCD — formats overlap; spot-check on import. |
| **Visual-only attrs** (`gamma`, `fps`, `flip`) | Drop silently; not deterministic state. |

---

## Validation plan

When implementation lands, validate against three corpora:

1. **Wokwi's own example gallery.** Scrape the public showcase, attempt import on each, score: clean / partial / blocked. Target: ≥70% clean for the boards in our mapping table.
2. **Top 20 GitHub repos with `diagram.json`.** Real user projects, not curated examples. Score same.
3. **Round-trip stability.** Import → re-export to Wokwi format → diff. The diff should contain only LabWired-specific additions (e.g., `external_devices` IDs) and visual-attr defaults, never functional changes.

---

## Phase-2 deliverables checklist

When this gets executed (after Device Library Phase 1):

- [ ] `packages/wokwi-import/` package with translator + tests
- [ ] Board map covering RP2040, ESP32-C3, ESP32-S3, STM32 Black Pill, Nucleo-F103RB
- [ ] Part map covering all Phase-1 device library entries
- [ ] `labwired import wokwi` CLI subcommand
- [ ] Playground "Import from Wokwi" button + file/URL input
- [ ] Import-report shows mapped/dropped/blocked counts and warnings inline
- [ ] CI fixture: import 20 real Wokwi projects, snapshot the resulting `system.yaml`
- [ ] Docs page `docs/tutorials/import-from-wokwi.md`

---

## Strategic take

Wokwi-import is *adoption fuel*, not a feature in itself. Most of the cost is owned by Phase-1 (the device library) and by future AVR support — the translator itself is a week of TypeScript once those are done.

The marketing line that opens up once this ships: **"Paste your Wokwi project URL. Run it on real silicon-parity firmware in CI."** That's the wedge into Wokwi's existing user base, and it directly leverages the HIL-displacement narrative that's already this project's positioning anchor.

Recommend revisiting this plan once Phase 1 is in Wave 4 — at that point the device library will tell us *exactly* which Wokwi parts are reachable and which aren't, and the board mapping table above can be turned into a concrete implementation backlog.
