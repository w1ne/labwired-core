# smart-basket-handle — the SmartBasket clip-on handle, in simulation

A shopping basket carries this handle: an **ESP32-S3** with a downward-facing
**OV3660** camera, an **ADXL345** accelerometer, an **HX711** load-cell ADC
and an **INA219** battery gauge. The device is a *sensor sled* — it captures
event-triggered JPEG bursts with weight and motion context and dumps the
session to the checkout station at the end of the trip. Recognition happens at
the station/cloud, never on the handle.

This example runs the real firmware under the LabWired simulator with all four
chip models attached from [`system.yaml`](system.yaml), and it is covered by
the end-to-end test `crates/core/tests/smart_basket_handle.rs`.

## Board wiring

| Function | Device | Bus / pins |
|---|---|---|
| Camera SCCB | OV3660 @ 0x3C | I2C1, SDA GPIO4 / SCL GPIO5 (board camera pads) |
| Camera DVP | OV3660 | LCD_CAM: XCLK 15, D0–D7 = 11 9 8 10 12 18 17 16, VSYNC 6, HREF 7, PCLK 13 |
| IMU | ADXL345 @ 0x53 | I2C0, SDA GPIO1 / SCL GPIO2 |
| Battery gauge | INA219 @ 0x40 | I2C0 (bus voltage = pack terminal, 4 mV/LSB) |
| Load cell | HX711 | DT GPIO21 (input), SCK GPIO47 (output), bit-banged |
| Checkout contact | DOCK | GPIO48 input; high = handle sits on a station |

Pins deliberately avoid the strapping pins (0/3/45/46), USB (19/20) and the
SPI-flash range (26–32). The camera pin map matches the
[`esp32s3-ov3660-camera`](../esp32s3-ov3660-camera) example, so the same
`ov3660` kit binds the sensor to LCD_CAM.

## Behaviour (all integer math — the simulator models no Xtensa FPU)

1. **Tare.** The first five HX711 samples establish the empty-basket baseline
   (debris or papers already inside are zeroed out). `TARE tare_g=…`
2. **Event detection.** Two streams run together: the accelerometer at 40 Hz
   and the load cell at 10 Hz with a 4-sample average. A settled weight change
   of ≥ 40 g against the last captured weight arms a capture; the firmware
   then waits for the filter to converge on the new weight (cap 1.5 s) and
   captures — so the recorded weight tracks ground truth, and the settle
   filter's own tail can never fake a second item. Raw motion without a weight
   change logs `ACCEL` and never captures (basket jostle, walking).
3. **Capture.** A two-frame JPEG burst (160×120, quality 12) through LCD_CAM +
   GDMA using the `ll_cam` register contract: drain one VSYNC edge, arm the
   ring on the next (the frame opens), stop on the one after it (the frame
   closes). Frames are stored with CRC-32 plus weight/accel-peak context.
   `EVT i=… t=… w=… dw=… a=… f0=… f1=…`
4. **Battery.** The INA219 bus voltage (pack terminal) is polled at 2 Hz.
   Below 3.30 V logs `LOW_BATT`, below 3.20 V logs `CRIT_BATT` and refuses new
   captures (`CRIT_SKIP`) — a POC policy, not a cell model.
5. **Checkout dump.** DOCK rising dumps the session over USB-Serial-JTAG:
   `DUMP events=N frames=M`, then per frame a `FRAME i=… len=… crc=0x…` header,
   `H <hex>` payload lines and `ENDFRAME i=…`, then `DUMP end`. DOCK falling
   resets the session for the next shopper. Hex (not raw bytes) because the
   JTAG printer can drop the tail of a burst whose FIFO is still draining —
   an ASCII stream re-syncs on newlines.

## Build and run

Requires the ESP Rust toolchain (see `examples/esp32s3-hello-world/README.md`)
and, for the simulator, a built `labwired` CLI.

```bash
cd examples/smart-basket-handle
cargo +esp build --release
# → target/xtensa-esp32s3-none-elf/release/smart-basket-handle

cd ../..   # repository root
cargo run -p labwired-cli -- run \
  --chip configs/chips/esp32s3.yaml \
  --system examples/smart-basket-handle/system.yaml \
  --firmware examples/smart-basket-handle/target/xtensa-esp32s3-none-elf/release/smart-basket-handle \
  --stop-on TARE
```

Interactive runs boot and tare, then wait for weight and motion stimulus
(which the CLI cannot drive on ESP32-S3 yet — use the test harness).

## Tests

```bash
cargo test --release -p labwired-core --features esp32s3-fixtures \
  --test smart_basket_handle
```

Two end-to-end tests, both running the **real firmware** with the board
attached from this example's `system.yaml`:

* `weight_events_battery_and_checkout_dump_end_to_end` — three placements
  (350 g, +550 g, −700 g) each produce one two-frame JPEG burst whose dumped
  bytes decode with `jpeg_decoder`, carry a matching CRC-32 and whose weight
  metadata matches the stimulus; a dimmed camera scene decodes measurably
  darker; the battery drops through LOW/CRIT, a critical pack refuses a 1200 g
  placement, and recovery re-enables captures; DOCK dumps all 6 frames and
  undocking resets the session.
* `imu_motion_without_weight_change_is_not_a_capture` — a jostled basket logs
  `ACCEL` and dumps an empty session.

Both are heavyweight (the camera renders and encodes synthetic JPEG frames in
simulated time, and the sim runs the full S3 peripheral set per cycle): the
pair takes roughly 20 minutes on a workstation. They are gated on the
`esp32s3-fixtures` feature like the other ESP end-to-end tests, so the default
workspace test run does not build firmware or need the ESP toolchain.

## What is and is not simulated

Simulated here: the firmware on the ESP32-S3; SCCB register traffic to the
OV3660; the DVP → LCD_CAM → GDMA → PSRAM/buffer path with the real JPEG bytes
the sensor model renders (synthetic scene: colour bars / brightness / motion
stimulus, not the OV3660's own compression engine); ADXL345 register reads
with stimulus in g; the HX711 bit-bang protocol with weight stimulus in
grams; INA219 bus voltage; the DOCK input; USB-Serial-JTAG console.

Not simulated (named rather than faked): fisheye optics, exposure/AWB/ISP,
image recognition (by design — the station/cloud does that), radio/WiFi
upload (the carrier draft's ESP-AT companion is a behavioral mock with no
data path), the charger/power-path electronics (see
[`smart-basket-power`](../smart-basket-power) for the reduced-order power
testbench and [`examples/smart-basket` in the app repo] for the carrier PCB),
and any physical/thermal behaviour. `HARDWARE.md` maps this system to a real
board.

## Console protocol (for tests and agents)

`BOOT` · `CAM pid=…` · `BATT mv=… soc=…` · `TARE tare_g=…` ·
`ACCEL t=… dev_sq=…` · `EVT i=… t=… w=… dw=… a=… f0=… f1=…` ·
`CRIT_SKIP t=… dw=…` · `LOW_BATT mv=… soc=…` · `CRIT_BATT mv=… soc=…` ·
`CAPTURE_FAIL t=…` · `DUMP events=… frames=…` · `EVENT i=…` · `FRAME i=…` ·
`H <hex>` · `ENDFRAME i=…` · `DUMP end` · `SESSION reset t=…`
