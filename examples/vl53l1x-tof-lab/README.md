# VL53L1X ToF Lab

STM32F103 + VL53L1X laser time-of-flight distance sensor over simulated I²C.

## What it does

1. Reads `IDENTIFICATION__MODEL_ID` (16-bit register `0x010F`) and prints it to
   UART — should be `0xEACC` (the value the Pololu driver's `init()` requires).
2. Starts ranging by writing `0x40` to `SYSTEM__MODE_START` (`0x0087`).
3. Polls `GPIO__TIO_HV_STATUS` (`0x0031`) for data-ready (`& 0x01 == 0`).
4. Runs a **continuous proximity monitor**: each loop reads the range status
   (`0x0089`) and the final range (`0x0096`/`0x0097`, 16-bit), back-converts the
   raw value the same way the driver does (`(raw * 2011 + 0x400) / 0x800`),
   classifies it against a **300 mm threshold**, and prints a line like
   `STATUS=9 RANGE=500 mm PROXIMITY=FAR` (or `PROXIMITY=NEAR` at/below the
   threshold) to UART1.

The threshold is the `PROXIMITY_THRESHOLD_MM` constant in `src/main.rs`
(300 mm): readings at or below it report `NEAR`, anything farther reports `FAR`.

The VL53L1X uses a **16-bit register pointer**, so each transaction writes the
register high byte then the low byte before the data/read phase.

## Interactive in LabWired

The model reports a **host-settable distance** (`Vl53l1x::set_distance_mm`,
default 500 mm). In the LabWired UI that value is driven live through the WASM
input bridge, so as you move the distance slider the firmware re-reads it every
loop and **`PROXIMITY` flips between `NEAR` and `FAR`** in real time around the
300 mm threshold — the lab is interactively dynamic, not a one-shot read.

## Building

```bash
cargo build -p vl53l1x-tof-lab --release --target thumbv7m-none-eabi
```

## Running the smoke test

```bash
cargo run -p labwired-cli -- test --script examples/vl53l1x-tof-lab/io-smoke.yaml
```

The headless smoke test runs at the default 500 mm distance and asserts the
`RANGE=500 mm PROXIMITY=FAR` decision; drive `set_distance_mm` from the UI to
exercise the `NEAR` path.
