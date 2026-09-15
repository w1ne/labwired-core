# Capacitive touch lab

Arduino Nano (ATmega328P) running the real `CapacitiveSensorSketch` from
[PaulStoffregen/CapacitiveSensor](https://github.com/PaulStoffregen/CapacitiveSensor)
(MIT), reduced to one sensor: send pin D4, receive pin D2, with a 1 MOhm
resistor from D4 to the touch pad node and the pad wired to D2. The onboard
LED (D13/PB5) lights when the reported `total=` count crosses `THRESHOLD`.

`system.yaml` models R1, the touch pad's own and finger-added capacitance,
and the D2/D4 drivers as an in-core analog co-simulation, matching what the
playground canvas compiler emits for the bundled starter diagram (see the
`cosim_models` comment in `system.yaml` — a drift test pins the two equal).

## Rebuilding the firmware fixture

CI does not build PlatformIO firmware for this lab, so the ELF is committed
at `../../tests/fixtures/avr/capacitive-touch-lab.elf`. Rebuild it with:

```bash
cd examples/capacitive-touch-lab
pio run
cp .pio/build/nanoatmega328/firmware.elf ../../tests/fixtures/avr/capacitive-touch-lab.elf
```

`platformio.ini` pins `PaulStoffregen/CapacitiveSensor` to commit
`aa0184827cf7da80225016842cf62692253bf347` (MIT license).

## Running the test

```bash
labwired test --script examples/capacitive-touch-lab/test.yaml
```

The script holds the touch pad released for 0.5 s of simulated time, then
presses it (`ui.touch.pressed = 1`), and asserts both a released-range and a
pressed-range `total=` reading plus the LED on at the end. The CLI test
`crates/cli/tests/capacitive_touch_lab.rs` additionally checks that the
median pressed `total=` is at least 3x the median released `total=`.
