# Raspberry Pi Pico W + BME280

This board-level system pairs the Raspberry Pi Pico W (SC0918) with an Adafruit
BME280 breakout (PID 2652). Firmware reads the sensor ID and raw temperature
registers through the modeled RP2040 I²C0 controller and reports the result on
UART0.

## Demand rank and availability

RP2040 is demand rank 5 in the measured MCU survey (12% usage). This is an I²C
sensor design on the existing Pico W board model, not a new MCU-family claim.

Availability was checked live on 2026-11-06. The exact Raspberry Pi SC0918
listing at DigiKey reports **Active**, **6,391 in stock**, and **$7.65** for
one unit. Adafruit's BME280 PID 2652 product page reports **In stock** at
**$14.95**. The URLs and observed states are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Wiring

Connect the breakout's SDA and SCL to Pico W GP4 and GP5, respectively. The
system profile selects I²C address `0x77`. Make the breakout's power and ground
connections according to its pinout and voltage requirements; electrical rails
and pull-up behavior are not simulated by this profile.

| Breakout signal | Pico W signal |
| --- | --- |
| SDA | GP4 |
| SCL | GP5 |
| Power | Use the breakout's documented supply input and the board's documented rail |
| GND | GND |

## Run the smoke

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id raspberry-pi-pico-w-bme280 \
  --crate firmware-pico-w-bme280-smoke \
  --target thumbv6m-none-eabi \
  --script examples/raspberry-pi-pico-w-bme280/onboarding-smoke.yaml \
  --system configs/systems/raspberry-pi-pico-w-bme280.yaml \
  --out-dir examples/raspberry-pi-pico-w-bme280/evidence
```

The smoke asserts `BME280_ID=0x60`, raw temperature bytes `0x082DC0`, the
completion marker, and a clean stop at the configured step limit. Build logs,
UART output, structured simulation result, and metrics are committed under
[`evidence/`](evidence/).

## Model scope

- Included and exercised: RP2040 startup from mapped flash, UART0 output,
  IO_BANK0 I²C pad selection, I²C0 register transactions, and the BME280 model.
- Omitted: CYW43439 Wi-Fi/Bluetooth, the CYW43439-driven onboard LED, USB/UF2,
  and board power behavior.
- The profile does not claim physical confirmation. The committed log is a
  local LabWired engine smoke; the physical bus voltage, pull-ups, and signal
  integrity are outside this simulation.
