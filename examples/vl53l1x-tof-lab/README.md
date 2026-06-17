# VL53L1X ToF Lab

STM32F103 + VL53L1X laser time-of-flight distance sensor over simulated I²C.

## What it does

1. Reads `IDENTIFICATION__MODEL_ID` (16-bit register `0x010F`) and prints it to
   UART — should be `0xEACC` (the value the Pololu driver's `init()` requires).
2. Starts ranging by writing `0x40` to `SYSTEM__MODE_START` (`0x0087`).
3. Polls `GPIO__TIO_HV_STATUS` (`0x0031`) for data-ready (`& 0x01 == 0`).
4. Loops reading the range status (`0x0089`) and the final range
   (`0x0096`/`0x0097`, 16-bit), back-converting the raw value the same way the
   driver does (`(raw * 2011 + 0x400) / 0x800`) and printing
   `STATUS= RANGE= mm` to UART1.

The VL53L1X uses a **16-bit register pointer**, so each transaction writes the
register high byte then the low byte before the data/read phase.

## Building

```bash
cargo build -p vl53l1x-tof-lab --release --target thumbv7m-none-eabi
```

## Running in LabWired

```bash
cargo run -p labwired -- test --script examples/vl53l1x-tof-lab/io-smoke.yaml
```

The model reports a host-settable distance (default 500 mm; see
`Vl53l1x::set_distance_mm`), mirroring the MPU6050 model's host-stimulus approach.
