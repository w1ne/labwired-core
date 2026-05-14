# Playground Device Library — Phase 1

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans`. Each device follows the worked template in [`2026-05-07-adxl345-sensor-lab-playground.md`](./2026-05-07-adxl345-sensor-lab-playground.md) — read that first.

**Goal.** Turn the playground from a GPIO-only canvas into a real "drag from library → wire → flash → run" simulator, with 8 protocol devices that cover I²C / SPI / UART / ADC. Match what users expect from Wokwi while preserving LabWired's silicon-parity story.

**Non-goals (Phase 1).**

- In-browser firmware compile. Keep shipping prebuilt ELFs from `packages/playground/public/wasm/`. (Iter 17.)
- Model registry / signing / `labwired install <model>`. (Iter 14.)
- Wokwi `diagram.json` import. (Separate plan — queued.)
- User-authored custom device YAML upload at runtime. (Phase 2.)
- 1-Wire, CAN, USB-device, RTOS-aware visualizers.

**Success metric.** A first-time visitor can pick any of the 8 devices from the palette, drop it on the canvas, hit Run, and see the device do something real over a real bus to a real firmware — no manual YAML editing.

---

## Device matrix

Ordered by ship sequence. "Editor" = SVG component in `packages/ui/src/editor/components/`. "Core" = simulator device in `core/crates/core/src/peripherals/components/`.

| # | Device | Bus | Address / pins | Editor today | Core today | Lab payoff |
|---|---|---|---|---|---|---|
| 1 | ADXL345 | I²C | 0x53 | Add | Add (planned) | Tilt sliders → axis chart |
| 2 | MPU6050 | I²C | 0x68 | Add | **Exists** (`mpu6050.rs`) | Combined accel + gyro |
| 3 | BME280 | I²C | 0x76 | Add | Add | Temp/humidity/pressure readout |
| 4 | SSD1306 OLED 128×64 | I²C | 0x3C | **Exists** (`oled-ssd1306.tsx`) | Add | Pixel grid renders from VRAM |
| 5 | MAX31855 | SPI | CS + SCK + MISO | Add | Add | Thermocouple temperature |
| 6 | ILI9341 TFT 240×320 | SPI | CS + DC + SCK + MOSI + RST | Add | Add | Framebuffer renders to canvas |
| 7 | NEO-6M GPS | UART | TX/RX | Add | Add | NMEA `$GPGGA` stream into firmware |
| 8 | NTC thermistor | Analog | ADC pin | Add | (analog input binding) | Slider → ADC value → temperature curve |

**Bus coverage after Phase 1:** I²C ×4, SPI ×2, UART ×1, ADC ×1.
**Visual payoff:** OLED bitmap + TFT color framebuffer (the demo "wow" pieces).

---

## Architecture decisions

### A1. Reuse the ADXL345 `external_devices` pattern for *all* buses.

The ADXL345 plan already adds `SystemBus::from_config` plumbing that walks `manifest.external_devices` and dispatches by `r#type` to `I2c::attach(Box<dyn I2cDevice>)`. Mirror that for SPI and UART:

- `SpiDevice` trait in `core/crates/core/src/peripherals/spi.rs` with `fn cs_select(&mut self)`, `fn transfer(&mut self, mosi: u8) -> u8`, `fn cs_release(&mut self)`.
- `UartPeripheral` gets a `push_rx(byte: u8)` API + optional attached "stream device" trait `UartStreamDevice` with `tick(&mut self) -> Option<u8>`.
- `Adc` gets a new `set_channel_input(channel: u8, millivolts: u16)` API hooked from board_io.

All four mechanisms (I²C attach already exists, SPI attach, UART push, ADC stimulus) are read-/written-from the same place: `manifest.external_devices` + `manifest.board_io` and exposed through analogous WASM bridge calls.

### A2. One YAML schema for every device.

```yaml
external_devices:
  - id: "screen"
    type: "ssd1306"           # device type identifier
    connection: "i2c1"        # peripheral id from chip descriptor
    config:
      i2c_address: 0x3C
      width: 128
      height: 64
```

Device-specific fields go under `config`. `connection` resolves to any peripheral; the device's trait (`I2cDevice` / `SpiDevice` / `UartStreamDevice`) determines which bus it can attach to.

### A3. `board_io` becomes the human-readable canvas binding.

Every visible device on the canvas gets a `board_io` entry that the WASM bridge uses to map UI events ↔ device state. Already used for ADXL345. Generalize the schema:

```yaml
board_io:
  - id: "screen"
    kind: "i2c_device" | "spi_device" | "uart_device" | "analog_input"
    peripheral: "i2c1"
    device_type: "ssd1306"
    # bus-specific identifiers
    i2c_address: 0x3C
    # OR
    spi_cs_pin: "PA4"
    # OR
    uart_id: "usart1"
    # OR (for analog)
    adc_channel: 0
```

### A4. Component palette grouped by bus.

`ComponentPalette.tsx` adds tabs: **GPIO**, **I²C**, **SPI**, **UART**, **Analog**, **Misc**. Each `ComponentDef` declares a `category` + `requiredBus` so dragging an I²C device onto a chip with no I²C peripheral surfaces a validation hint instead of an opaque YAML failure.

---

## Cross-cutting prerequisites (Task 0)

These land before or alongside Wave 1. Each is its own commit.

- [ ] **0a. Formalize the `external_devices` schema.** Add JSON-schema in `core/configs/schemas/external_device.schema.json`; update `labwired-config` crate to validate. Reject unknown `type` with actionable error.
- [ ] **0b. SPI device-attach plumbing.** `SpiDevice` trait in `core/crates/core/src/peripherals/spi.rs`. `Spi::attach(Box<dyn SpiDevice>)`. CS-pin matching driven by board_io. Unit test: dummy device echoes MOSI byte+1 back as MISO.
- [ ] **0c. UART RX injection.** `Uart::push_rx(byte)` + `UartStreamDevice` trait with `tick(&mut self, elapsed_us: u32) -> Option<u8>`. Bus plumbing attaches stream devices declared in `external_devices`. Unit test: counter device emits 0,1,2,... on each tick; firmware reads RDR sees them.
- [ ] **0d. ADC stimulus from board_io.** `Adc::set_channel_input(channel, millivolts)` + WASM bridge `set_adc_input(device_id, millivolts)`. Board_io `kind: "analog_input"` binds a canvas widget (slider, curve sim) to an ADC channel.
- [ ] **0e. Component palette categories.** `ComponentDef.category: 'gpio' | 'i2c' | 'spi' | 'uart' | 'analog' | 'misc'`. `ComponentPalette.tsx` groups by category. Validation warns when a device's required bus is absent from the current chip.
- [ ] **0f. Visual feedback layer.** Generic `DeviceVisualizer` slot in `GuidedLab`/workbench inspector so each device renders its widget (chart, pixel grid, NMEA log, thermometer) without bespoke playground glue.

**Order:** 0a → 0b/0c/0d in parallel → 0e → 0f.

---

## Wave 1 — I²C breadth (Weeks 1–2)

**Goal:** prove the template, ship the two simplest sensors, land the first display.

### 1. ADXL345 — *canonical first device*

Already fully specified in [`2026-05-07-adxl345-sensor-lab-playground.md`](./2026-05-07-adxl345-sensor-lab-playground.md). Execute that plan as-is — it produces the device, the bus plumbing (which Task 0a generalizes immediately after), the WASM bridge pattern, the editor component, the bundled lab. Use this implementation as the *reference* for every device that follows.

### 2. MPU6050

- Core: `core/crates/core/src/peripherals/components/mpu6050.rs` — already present as a stub. Complete `I2cDevice` impl: WHO_AM_I=0x68, ACCEL_XOUT_H/L (0x3B–0x40), GYRO_XOUT_H/L (0x43–0x48), PWR_MGMT_1 (0x6B).
- Editor: `packages/ui/src/editor/components/mpu6050.tsx` — 6-axis breakout SVG.
- Bridge: extend `setI2cSensorSample` to accept `(ax, ay, az, gx, gy, gz)` for `kind: 'mpu6050'`; extend `I2cSensorState` discriminator.
- Lab preset: `core/configs/systems/mpu6050-lab.yaml`, demo firmware at `core/examples/mpu6050-lab/`.
- UI: extend `Adxl345Visualizer` → generic `Imu6Visualizer` with two charts (accel + gyro) so both ADXL345 and MPU6050 reuse it.

### 3. BME280

- Core: `bme280.rs` — model the calibration coefficient registers (0x88–0xA1 + 0xE1–0xE7). Expose `set_environment(temp_c: f32, humidity_pct: f32, pressure_hpa: f32)`; internally compute raw `temp_raw`/`hum_raw`/`press_raw` so firmware's compensation math returns the right value. **This is the highest-effort I²C device** because of the calibration math — budget 2× the ADXL345 effort.
- Editor: weather-station breakout SVG.
- Bridge: `setEnvironment(deviceId, temp, humidity, pressure)`.
- Lab preset: STM32F103 + BME280 weather station with sliders for each axis.
- Test fixture: real Bosch driver reads producing expected outputs within ±0.5 °C.

### 4. SSD1306 OLED 128×64 (I²C variant)

- Editor: **already exists** (`oled-ssd1306.tsx`). Verify pin labels.
- Core: `ssd1306.rs` — track command-mode flag (control byte 0x00 vs 0x40), display data RAM (1024 bytes = 128×8 pages of 8 vertical bits). Support `SET_COLUMN_ADDR` (0x21), `SET_PAGE_ADDR` (0x22), `DISPLAY_ON` (0xAF). Expose `framebuffer() -> &[u8; 1024]`.
- Bridge: `getSsd1306Framebuffer(deviceId): Uint8Array`. Convert 8-pixel-vertical pages to a 128×64 1bpp bitmap.
- UI widget: `Ssd1306Display` React component, 128×64 `<canvas>` redrawing on each frame.
- Lab preset: STM32F103 prints "LabWired" + frame counter to the OLED via I²C.
- **This is the first device with a visible non-trivial output.** Treat it as a demo flagship.

**Wave 1 exit criteria:** four I²C devices on the canvas, generic `Imu6Visualizer` + `Ssd1306Display` widgets, one common WASM bridge entry point per kind, schema validation rejecting unknown I²C device types.

---

## Wave 2 — SPI bus (Weeks 3–4)

### 5. MAX31855

- **Land Task 0b (SPI attach plumbing) here.** MAX31855 is the simplest SPI device — pure read, no command bytes, 4-byte response — so it doubles as the SPI plumbing's TDD harness.
- Core: `max31855.rs` — 32-bit big-endian response: bits [31:18] = thermocouple °C × 4 (14-bit signed), bit 16 = fault, bits [15:4] = internal °C × 16, bits [2:0] = OC/SCG/SCV. Expose `set_temperature(tc_c: f32, internal_c: f32)`.
- Editor: thermocouple module SVG with screw terminals.
- Bridge: `setThermocouple(deviceId, tc, internal)`.
- Lab preset: STM32F401 + MAX31855 → UART prints temperature each second.

### 6. ILI9341 TFT 240×320

- Core: `ili9341.rs` — DC pin distinguishes command vs data byte (this is what Task 0b's `SpiDevice` trait must expose: a sideband signal map). Support `CASET` (0x2A), `PASET` (0x2B), `RAMWR` (0x2C), `MADCTL` (0x36), `COLMOD` (0x3A) for 16bpp RGB565. Allocate a 240×320×2 = 153600-byte framebuffer.
- Editor: 2.4" TFT module SVG (orange PCB, classic 320×240 look).
- Bridge: `getIli9341Framebuffer(deviceId): Uint8Array` (RGB565 little-endian).
- UI widget: `Ili9341Display` renders to a 240×320 `<canvas>` by mapping RGB565 → ImageData. Throttle to 30 fps.
- Lab preset: STM32F401 + ILI9341 draws a moving gradient (cheap firmware, looks impressive). **This is the demo-day flagship.**

**Wave 2 exit criteria:** SPI device-attach plumbing in core; two SPI devices shipping; the TFT framebuffer rendering smoothly in the browser at 30 fps; schema validates `spi_cs_pin` against the active chip's GPIO map.

---

## Wave 3 — UART + ADC (Week 5)

### 7. NEO-6M GPS

- **Land Task 0c (UART RX injection) here.**
- Core: `neo6m.rs` implements `UartStreamDevice`. Internal NMEA generator builds `$GPGGA` / `$GPRMC` sentences from a `(lat, lon, fix_age)` state with NMEA checksum. Tick rate = 1 Hz default.
- Editor: GPS breakout SVG with antenna icon.
- Bridge: `setGpsPosition(deviceId, lat, lon)` + `setGpsFix(deviceId, fix: 'none' | '2d' | '3d')`.
- UI widget: `GpsMap` with a draggable pin (no real map tiles — just a grid + coords) + NMEA console.
- Lab preset: ESP32-S3 or STM32F401 + NEO-6M parses NMEA, prints lat/lon to UART.

### 8. NTC thermistor (analog)

- **Land Task 0d (ADC stimulus) here.**
- Core: no new device file — analog input is a board_io binding, not an attached device. Add `Adc::set_channel_input(channel, millivolts)`.
- Editor: thermistor SVG with two leads + temperature slider widget.
- Bridge: `setAdcInput(deviceId, millivolts)`.
- UI widget: `Thermometer` — temperature slider that converts °C → expected ADC mV using the Steinhart-Hart curve for a 10k NTC + 10k divider at 3.3 V, then pushes the mV to the bridge.
- Lab preset: STM32F103 reads ADC1_IN0, applies the same Steinhart-Hart curve in firmware, prints °C to UART. Slider value and printed value must match within 0.1 °C.

**Wave 3 exit criteria:** UART RX injection from board_io; ADC stimulus from board_io; full bus matrix (I²C / SPI / UART / ADC) covered.

---

## Wave 4 — Polish & launch (Week 6)

- [ ] **Palette categories.** Land Task 0e if not already.
- [ ] **Validation.** Dragging an I²C device onto a chip without I²C peripheral surfaces an inline hint ("This chip has no I²C — add the BME280 to a chip with `i2c1`."). Wire-pin mismatches (SDA on a non-SDA pin) surface yellow underline.
- [ ] **Starter diagrams.** For each device, add a one-click "Load lab" preset that drops the device, wires it correctly to the MCU, and selects matching demo firmware.
- [ ] **DEMOS.md update.** Replace the GPIO-blinky lede with one I²C + one SPI demo. Record 30-second GIFs of OLED and TFT labs.
- [ ] **Catalog page update.** `docs/specs/compatibility_matrix.md` gets a "Peripheral devices" section listing the 8 + the existing 24 GPIO components.
- [ ] **End-to-end agent test.** `ai/tests/` exercise: agent assembles a BME280 weather station via tool calls (drop component → wire → set environment → run → assert UART contains expected temp). Validates that the canvas + agent paths produce the same `external_devices` YAML.

---

## File-touch summary (per device)

For each new device, the change set is the same shape:

| Location | Change |
|---|---|
| `core/crates/core/src/peripherals/components/<dev>.rs` | New file: device model + `*Device` trait impl |
| `core/crates/core/src/peripherals/components/mod.rs` | `pub mod`/`pub use` line |
| `core/crates/core/src/bus/mod.rs` | Dispatch case for the new `type` |
| `core/crates/wasm/src/lib.rs` | Bridge methods (`set_*` / `get_*`) |
| `packages/ui/src/wasm/simulator-bridge.ts` | Typed wrappers + state type |
| `packages/ui/src/editor/components/<dev>.tsx` | SVG component + pin map |
| `packages/ui/src/editor/components/index.ts` | Register component |
| `packages/ui/src/components/<DevWidget>/` | Visualizer / control widget |
| `packages/ui/src/index.ts` | Export widget |
| `core/configs/systems/<dev>-lab.yaml` | System manifest |
| `core/examples/<dev>-lab/` | Firmware crate |
| `core/Cargo.toml` | Workspace + release profile |
| `packages/playground/public/wasm/demo-<dev>.elf` | Built artifact |
| `packages/playground/src/bundled-configs.ts` | Board entry + starter diagram |
| `packages/playground/src/bundled-configs.test.ts` | Assertion |
| `docs/specs/compatibility_matrix.md` | Catalog row |

Per-device budget: **2–4 days** for I²C/UART/ADC devices; **5–7 days** for displays (framebuffer + canvas rendering) and BME280 (calibration math).

---

## Risks & open questions

1. **Component visual fidelity.** Wokwi has hand-illustrated, recognizable parts. Our current SVGs are stylized monochrome. Phase 1 ships *functional* SVGs; raise visual quality in a follow-up after the bus matrix is closed.
2. **Wokwi ecosystem gravity.** Many of the firmware examples our users will paste in were written against Wokwi pin labels. Decision: ship Wokwi-import compat as a separate plan (see next deliverable).
3. **Cycle accuracy vs. simulation budget.** ILI9341 framebuffer churn at 30 fps × 240×320×2 B = 4.6 MB/s through SPI. Need to confirm the WASM simulator can sustain the firmware-side SPI traffic without stalling. Spike test in Wave 2 before fully committing to ILI9341 — fall back to ST7735 128×128 if budget blows.
4. **Schema versioning.** Once `external_devices` lands as a stable schema, every new device is an additive change. Lock `schema_version: "1.0"` for the external_devices block before Wave 1 ships externally.
5. **`labwired_ai` discoverability.** The agent path needs `list_supported_devices()` and `describe_device(type)` so an LLM agent doesn't hallucinate `type: "bmp280"` when we shipped `bme280`. Add to Wave 4.

---

## Self-review

- **Spec coverage.** All 8 devices + 4 cross-cutting prereqs are scoped. The lean-scope principle (`feedback_minimal_scope`) is honored — no Phase 2 work (compile, registry, Wokwi import, custom YAML) is included.
- **Template reuse.** Every device follows the same 16-file change set; the ADXL345 plan is the worked example.
- **Bus matrix.** I²C, SPI, UART, ADC all covered; 1-Wire / CAN / USB explicitly deferred.
- **Demo value.** Two visual flagships (OLED + TFT) carry the demo. Sensor variety (motion / environment / temperature / location) covers the most common embedded use cases.
- **Agent compatibility.** External device schema + board_io are both YAML-first, so the canvas and the agent path emit the same artifact.
