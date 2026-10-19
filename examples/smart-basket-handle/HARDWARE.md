# SmartBasket handle — hardware mapping

This example simulates the device the Pickflow SmartBasket specs describe: a
clip-on handle ("smarthandle") that turns any standard basket into a
checkout-ready sensor sled. This file maps the simulated system to a physical
board so the two cannot drift.

## Pin map (simulated == prescribed board)

| ESP32-S3 pin | Signal | Notes |
|---|---|---|
| GPIO4 / GPIO5 | Camera SDA / SCL (SCCB) | board camera pads; I2C1 |
| GPIO15 | Camera XCLK | LEDC or inverted APLL clock on real boards |
| GPIO11, 9, 8, 10, 12, 18, 17, 16 | Camera Y2…Y9 (D0–D7) | DVP data |
| GPIO6 / GPIO7 / GPIO13 | VSYNC / HREF / PCLK | DVP sync |
| GPIO1 / GPIO2 | IMU + gauge SDA / SCL | I2C0 |
| GPIO21 / GPIO47 | HX711 DT (input) / SCK (output) | 10 Ω series on SCK recommended |
| GPIO48 | DOCK input | station contact, active high |
| GPIO19 / GPIO20 | USB D−/D+ | programming + charging on real boards |
| GPIO26–32 | SPI flash | reserved |
| GPIO0/3/45/46 | strapping | reserved |

## Bill of materials (POC handle)

| Part | Selected device | Interface |
|---|---|---|
| MCU module | ESP32-S3-WROOM-1 N16R8 (16 MB flash, 8 MB PSRAM) | — |
| Camera | OV3660 (or OV2640 for 160×120 POC frames) + fisheye M12/board lens | DVP + SCCB |
| IMU | ADXL345 (or LSM6DS3-class 6-axis) | I2C 0x53 |
| Load cell ADC | Avia HX711 + 5 kg bar/button cell ($<1 for the cell) | 2-wire bit-bang |
| Battery gauge | INA219 (or MAX17048 fuel gauge) | I2C 0x40 |
| Battery | 1S LiPo, ~2000 mAh, protected, with NTC | JST-PH, keyed |
| Charger | TP4056-class with power path (e.g. BQ24074) + load switch | see carrier draft |
| DOCK contact | spring pin / hall sensor on the station cradle | GPIO48 |

The spec's production target is $18–33 per module at volume; this POC
selection sits inside the earlier $36–58 bracket because it uses
module-level parts.

## Power budget (POC)

| Mode | Current @ 3.7 V | Notes |
|---|---|---|
| Idle (poll IMU/load cell, camera stopped) | ~40–60 mA | ESP32-S3 + ADXL345 + HX711 |
| Capture burst (2 JPEG, 160×120, ~3500 B each in the model) | ~160 mA for ~0.3 s | camera + DMA + flash |
| Checkout dump (session, 6 frames) | ~120 mA for ~2 s | radio/flash dominated |

A 2000 mAh cell therefore covers well over a shopping day of event-driven
use; the simulator does not model the cell or the charger — that reduced-order
power system lives in [`smart-basket-power`](../smart-basket-power) and the
carrier PCB in the app repo's `examples/smart-basket`.

## What the physical board must add that the simulation cannot prove

1. **Fisheye optics and basket geometry** — field of view, occlusion by
   stacked items, the lens bump that tells shoppers where not to cover.
2. **Charger/power path qualification** — BQ24074-class charging, rail
   sequencing, low-battery supervisor behaviour (the firmware's 3.30 V/3.20 V
   thresholds are POC policy, not cell measurements).
3. **USB VBUS isolation** for programming while battery-powered.
4. **Fixture mechanics** — the snap-fit adapter ("baskadapter") for the
   common basket handle shapes, and the anti-theft tether.
5. **Radio** — the real upload link at checkout; the model stops at the
   session dump over the host link.
6. **Environmental** — drops, cleaning, −10 °C…35 °C, 15 kg overload.

## Continuity with the carrier draft

The app repo's `examples/smart-basket` carrier (Luckfox Pico Mini A + ESP32-C3
ESP-AT + ADXL345 + BQ24074 power path) is the complementary artifact: it owns
the *power-path and PCB* evidence (ERC/DRC, routing, ngspice divider checks,
the native Linux joint run). This example owns the *device pipeline* evidence
(camera, IMU, load cell, battery gauge, session protocol). Where they overlap
(sensor pin maps, ESP-AT radio role, the 3.3 V rail), the numbers above are
the shared contract.
