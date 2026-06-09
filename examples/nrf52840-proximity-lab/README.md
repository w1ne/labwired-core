# nRF52840 + HC-SR04 Ultrasonic Proximity Lab

A complete, runnable LabWired lab: an **nRF52840** firmware reads an **HC-SR04**
ultrasonic distance sensor over GPIO, converts the echo pulse to a distance,
raises an **ALARM** output when the target is within a 150 mm threshold, and
**broadcasts {distance, in-range} over the BLE 1 Mbit radio** — the same "set a
target distance, watch the flag go high, send it to your phone" loop you would
build on the physical board.

The firmware talks only to raw nRF GPIO + RADIO registers (no LabWired APIs), so
the **same ELF runs in the simulator and flashes to real silicon**.

## Wiring

| Signal | Pin    | Direction        | Notes                                  |
|--------|--------|------------------|----------------------------------------|
| TRIG   | P0.04  | MCU output       | ≥10 µs pulse starts a ranging          |
| ECHO   | P0.05  | MCU input        | sensor holds it high ∝ distance        |
| ALARM  | P0.06  | MCU output       | the "in range" flag (LED/buzzer)       |

The distance and in-range flag are also transmitted over BLE (1 Mbit PHY,
FREQUENCY=42) as a 4-byte payload `{distance_mm LE, in_range, counter}` — open
the Air Tracer / Packet Analyzer to watch them, or receive on a phone.

The HC-SR04 is GPIO-wired (not an I2C/SPI bus device). The simulator services
it each tick: it watches the TRIG output and drives the ECHO input high for
`distance_cm × 58 µs`, exactly like the real module. `distance_cm` in the
system manifest is the host-controlled "hand position."

## Build the firmware

```bash
cargo build -p firmware-nrf52840-proximity --release --target thumbv6m-none-eabi
```

## Run it

The lab ships two deterministic scenarios that differ only by `distance_cm` in
the system manifest (same firmware ELF):

```bash
FW=target/thumbv6m-none-eabi/release/firmware-nrf52840-proximity

# Target at 10 cm -> inside 150 mm -> ALARM high
labwired test -f "$FW" -c examples/nrf52840-proximity-lab/proximity-near.test.yaml

# Target at 30 cm -> outside 150 mm -> ALARM low
labwired test -f "$FW" -c examples/nrf52840-proximity-lab/proximity-far.test.yaml
```

Both exit `0` when every assertion passes.

## What the run proves

The firmware exposes its results at fixed RAM symbols (`DISTANCE_MM`,
`IN_RANGE`, `LAST_TICKS`, `TX_DONE_COUNT` at `0x2000_0000…`), and the ALARM line
is the nRF P0 `OUT` register (`0x5000_0504`, bit 6). The test scripts assert all
of them:

| Scenario        | DISTANCE_MM | IN_RANGE | ALARM (P0.06) | BLE TX |
|-----------------|-------------|----------|---------------|--------|
| Near (10 cm)    | 100         | 1        | high (0x40)   | 234    |
| Far  (30 cm)    | 299         | 0        | low  (0x00)   | 95     |

`TX_DONE_COUNT` is the number of full RADIO transmit handshakes
(TXEN→END→DISABLE) the firmware completed — proof the BLE broadcast actually ran
on the modeled radio, not just that a buffer was filled.

> **Browser note:** the wasm build in the playground uses a slightly different
> instruction-cycle model than the native CLI, so the in-browser distance reads
> a little high for the same hand position. The CLI numbers above are the
> calibrated, exact ones; the browser is a faithful live demo of the same loop.

The distance read-back (100 mm / 299 mm) confirms the firmware actually
*consumed the ECHO timing through the modeled GPIO registers* and computed a
real distance — not that a value was merely injected. The 299 (vs 300) is
integer-division rounding, and is deterministic.

## Calibration

The firmware times the ECHO pulse as a count of its own polling-loop iterations
(`LAST_TICKS`) and compares it to `THRESHOLD_TICKS`, the count that corresponds
to 150 mm. That count is deterministic for a given HC-SR04 `cpu_hz` (64 MHz
here) and this firmware's measurement loop; it was calibrated by running at
`distance_cm: 15.0` (= 150 mm) and reading `LAST_TICKS` (8950). On real silicon
you would instead measure the pulse with a hardware timer; the register-level
interaction is identical.

## Files

- `system.yaml` / `system-far.yaml` — machine manifests (chip + HC-SR04 at 10 / 30 cm)
- `proximity-near.test.yaml` / `proximity-far.test.yaml` — assertions
- firmware: `crates/firmware-nrf52840-proximity/`
