# ESP32-C3 Leo air-quality sensor

A simulated home air-quality monitor: an **ESP32-C3** reads four air-quality
sensors over the real C3 I²C0 controller and turns the raw measurements into a
**plain-language verdict** over UART — "air quality is good" → "CO₂ climbing,
crack a window" — live, with no hardware in the loop.

The point of this example is that the sensor firmware is the *real thing*: three
of the four drivers are the **unmodified Sensirion `embedded-i2c` vendor
libraries** running on-target (riscv32), plus Sensirion's **Gas Index
Algorithm**. They issue genuine I²C transactions against behavioral sensor
models that answer with datasheet-correct words and CRCs, so the same firmware a
team would flash to real silicon boots and decodes here.

## The board

| Sensor    | Part            | Bus / addr | Metric                         | Driver |
|-----------|-----------------|------------|--------------------------------|--------|
| CO₂       | Sensirion SCD41 | I²C0 0x62  | CO₂ ppm, temperature, humidity | real `embedded-i2c-scd4x` |
| VOC       | Sensirion SGP41 | I²C0 0x59  | VOC raw → VOC Index            | real `embedded-i2c-sgp41` + `gas-index-algorithm` |
| Particles | Sensirion SPS30 | I²C0 0x69  | PM1/2.5/4/10                   | real `embedded-i2c-sps30` (uint16 output) |
| Light     | Vishay VEML7700 | I²C0 0x10  | ambient lux                    | register-level driver (Vishay ships no bare-metal C lib) |
| Surface T | Melexis MLX90614| I²C0 0x5A  | IR surface temperature (°C)    | bare-C SMBus driver (read-word + CRC-8 PEC) |
| Screen    | SSD1306 OLED    | I²C0 0x3C  | 128×64 on-device display       | bare-C driver + 5×7 font |

That covers every metric on the product brief — CO₂, particulates, VOC,
humidity, light, temperature — on one I²C bus, plus a non-contact IR surface
temperature for condensation, and a 128×64 OLED that shows the plain-language
verdict on the device itself.

## The screen

The firmware renders the readings and the headline verdict to the SSD1306 OLED
each cycle (the playground draws the live panel). For headless runs it also
echoes the final frame as ASCII art between `OLED-FB-BEGIN`/`OLED-FB-END` so the
rendered screen is verifiable in the log. A normal-scenario frame:

```
LEO AIR QUALITY
CO2  1395 PPM
PM2.5 22 UG
VOC 0
LIGHT 91 LX
TEMP 23C RH 68%
MOLD  HIGH
>MOLD RISK HIGH
```

(the stuffy scenario shows `>VENTILATE NOW`).

## Mold Index

Beyond the raw readings the firmware derives a **Mold Index** — the headline
metric commercial IAQ monitors report and the one Leo's mold-detection use case
is built around. Mold is not a sensor reading: it germinates when humidity stays
high in a livable temperature band, and the longer a room sits damp the higher
the risk. The firmware tracks a dwell counter of consecutive mold-favorable
cycles and combines it with the humidity level — entirely from the SCD41's
temperature and humidity. As the closed room's humidity climbs past ~60 %RH the
verdict escalates `low → watch → ELEVATED → HIGH` and takes over the OLED
headline.

### Surface condensation — the moisture-first upgrade

Air humidity lies about the wall. Mould starts on the **cold surface** (a window,
an exterior wall, a thermal bridge), where the *surface* relative humidity can be
at condensation (100 %) while the room air still reads a benign ~70 %. The
MLX90614 IR thermometer reads that surface temperature; with the SCD41's air
temperature and RH the firmware computes the **dew point** and the **surface RH**
from the Magnus saturation-vapour relation (real `expf`/`logf`, soft-float on the
FPU-less C3) and flags condensation when the wall drops below the dew point.

That surface signal escalates the mold verdict for a reason an air-only "Mold
Index" is blind to. In the default scene the wall cools from 18 °C toward 10 °C
while the air RH climbs into the 70s; partway through, the surface crosses the
dew point and the run prints:

```
SURFACE: 10.03C dew 17.68C surfaceRH 164% CONDENSING - wall is wet
MOLD: mold risk: HIGH - sustained damp (surface condensation)
```

The `system-stuffy.yaml` scene (cold exterior wall, damp room) drives this to
`SEVERE` early and holds it there; `system-fresh.yaml` (warm surface, dry air)
never condenses — the negative control. Note that the NORMAL scene's air-RH
ladder asymptotes at 70.1 %, a hair over the firmware's 70 % "very damp" step, so
NORMAL also tips from `HIGH` to `SEVERE` in its last few samples. The mold
verdict therefore separates fresh from the other two, but not normal from stuffy;
**CO₂ is what separates NORMAL from STUFFY** (1394 ppm vs 1791 ppm, either side
of the 1400 ppm "ventilate now" threshold). This is the differentiating channel from the Leo "moisture
first" product direction, exercised end-to-end over the real C3 I²C bus.

## How it works

The three Sensirion parts speak Sensirion's command protocol: a 16-bit
big-endian command, responses as 16-bit words each followed by a CRC-8
(polynomial 0x31). The device models (`crates/core/src/peripherals/components/`)
implement the real command sets and encode CRC-correct words, so the unmodified
vendor drivers decode them exactly. The platform shim
(`firmware/sensirion_i2c_hal_c3.c`) implements Sensirion's five I²C HAL hooks by
driving the C3 I²C0 command-list engine directly — every byte the driver reads
is fetched by a real simulated I²C transaction.

The numbers **move**, and there is exactly one thing that moves them: the
`stimuli:` ladder in the test script, applied through `set_input`. The sensor
models hold whatever value they were last given — they do **not** self-advance a
ramp. So a closed room fills up — CO₂ climbs from ~450 toward ~1400 ppm,
particulates drift up, the light dims toward evening — because the script drives
it, and the firmware's verdict flips. The verdict thresholds are firmware policy,
blind to the scene config.

Each ladder rung is an absolute cycle count (`after_cycles`), so the rungs must
land inside the window in which the firmware is still sampling. Measured against
the committed ELF: one measurement cycle costs ~343.8k core cycles, sample `t=4`
is read at 1.25M cycles, sample `t=60` at 20.5M, and all 64 samples plus the
final OLED frame are done by ~8.65M steps / ~22.6M cycles. A rung placed past
that window is dead — it changes no reading, and the scenario it belongs to then
passes or fails for reasons unrelated to the story it claims to tell.

> Note: the SGP41 VOC Index reads 0 for the first ~45 cycles. That is the **real**
> Gas Index Algorithm's warm-up/blackout behaviour, not a stub — it gates output
> until its adaptive baseline settles. CO₂ is the headline metric.

## Scenarios

| Manifest               | Story                          | Verdict arc                              |
|------------------------|--------------------------------|------------------------------------------|
| `system.yaml`          | closed room fills up (NORMAL)  | fresh → "crack a window"                 |
| `system-stuffy.yaml`   | crowded, poorly ventilated     | climbs past 1400 ppm → "ventilate now"   |
| `system-fresh.yaml`    | well-ventilated room           | stays "air quality is good"              |

## Build

Needs the Espressif RISC-V GCC toolchain (`riscv32-esp-elf-gcc`, from PlatformIO
or ESP-IDF). A pre-built `firmware/leo_airquality.elf` is committed so the
example runs without the toolchain.

```sh
make -C examples/esp32c3-leo-airquality/firmware
```

## Run

```sh
cargo run --release -p labwired-cli -- test \
  --script examples/esp32c3-leo-airquality/test.yaml          # NORMAL
cargo run --release -p labwired-cli -- test \
  --script examples/esp32c3-leo-airquality/test-stuffy.yaml   # ventilate now
cargo run --release -p labwired-cli -- test \
  --script examples/esp32c3-leo-airquality/test-fresh.yaml    # stays fresh
```

Sample NORMAL output:

```
LEO BOOT
SCD41 READY
SGP41 READY
SPS30 READY
VEML7700 READY
t=0  CO2=526ppm  T=22.05C RH=45% PM2.5=7ug  VOC=0  LUX=421
AIR: fresh - air quality is good
...
t=10 CO2=1020ppm T=22.54C RH=47% PM2.5=16ug VOC=0  LUX=246
AIR: getting stuffy - CO2 climbing, crack a window; some haze in the air
...
LEO DONE
```

## Model limitation

The C3 I²C controller clocks each transaction bit-by-bit at the wire rate its
timing registers dictate, into a 32-byte RX FIFO (silicon-accurate). Every read
on this board fits one command-list run (data-ready 3 B, SCD4x measurement 9 B,
SGP41 raw 6 B, SPS30 uint16 30 B). The SPS30's 60-byte *float* frame would not
fit a single run's FIFO, so this example drives the SPS30 in its 30-byte integer
output mode — still the real vendor driver, real command codes, real CRCs.
