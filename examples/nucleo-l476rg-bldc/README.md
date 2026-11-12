# STM32L476 six-step BLDC control

This example runs real bare-metal STM32L476RG firmware against LabWired's
deterministic BLDC plant. TIM1 supplies three main/complementary PWM pairs with
dead time and master-output gating; PB0 is the external inverter enable.

The plant uses 24 V, 0.35 Ω phase resistance, 180 µH phase inductance,
0.04 N·m/A torque constant, 0.04 V/(rad/s) back-EMF constant, 0.1 µkg·m² rotor
inertia, seven pole pairs, and a 2048-count encoder. Encoder A/B/index feed
PC3..PC5 and Hall A/B/C feed PC0..PC2. The distinct
active-high safety inputs are undervoltage PC7, overcurrent PB6, and
inverter/driver PB7. PC6 is reserved for a future aggregate motor fault but is
deliberately unbound: mechanical stall is diagnosed from missing Hall motion,
so an open-phase/aggregate fault cannot be mislabeled as `FAULT STALL`. TIM1 main outputs use PA8/PA9/PA10 and their
complementary outputs use PB13/PB14/PB15.

The firmware begins with open-loop duty, selects a six-step row from the Hall
sequence `001 → 101 → 100 → 110 → 010 → 011`, and uses a deliberately small
bounded proportional regulator to adjust duty from Hall edge period. A 100 µs
SysTick is the authoritative controller clock. Closed-loop operation starts
only after three sequential Hall edges; `TARGET REACHED` requires two
consecutive measured Hall periods in the 0.4–1.2 ms band plus observed encoder
motion. Invalid-Hall, stationary, overcurrent, undervoltage, and driver-fault
counters are bounded. Every fault handler first clears TIM1 `BDTR.MOE`,
disables all CCER legs, and drops PB0 before printing.

```bash
cargo build --release -p firmware-l476-demo \
  --bin firmware-l476-bldc-six-step --target thumbv7em-none-eabihf
cargo run -q -p labwired-cli -- test \
  --script examples/ci/l476-bldc-stall.yaml \
  --output-dir out/l476-bldc
```

The UART sequence is `BLDC READY`, `TARGET REACHED`, `FAULT STALL`, and
`INVERTER OFF`. The CI script injects a real mechanical stall through the
generic motor input after target acquisition; it does not forge UART output.
The run has a 310,000-cycle (3.875 ms) post-injection ceiling; the reference run
shuts down well inside that bound. The scenario selects one-based stimulus and
UART occurrences explicitly. Its `result.json` assertion evidence records the
actual successful stimulus cycle, first qualifying UART-token cycle, measured
latency, and configured maximum; this remains valid when assertion early-stop
is disabled. Latency assertions force UART observation after every retired
instruction, so event cycles are instruction-boundary observations rather than
an overclaim about the exact internal peripheral write cycle.

For robotics startups this moves repeatable commutation, startup, Hall-order,
and shutdown regressions into CI while retaining the same MCU binary and
register-level safety path used on hardware.

This is a reduced-order six-step plant: it models phase R/L, trapezoidal
back-EMF, shaft inertia/load, Hall sensors, encoder feedback, bus
undervoltage, and a latched phase-current threshold. It does not model MOSFET
switching transients, thermal behavior, magnetic saturation, field-oriented
control, or mechanical drivetrain/contact dynamics.

## Scriptable inverter fault (LabWired Core)

The same inverter fault exposed by Studio can now be driven in a headless
regression through the motor SimInput channel named inverter_fault. Use the
drive_motor component ID, and a boolean value 0.0 (clear) or 1.0 (assert).
The motor's normal physical fault pin remains connected to firmware input PB7.

The fixture in examples/ci/l476-bldc-inverter-fault.yaml injects the fault
at cycle 1,500,000. It requires the real STM32L476 ELF to report target
speed before the fault markers, then checks motor-off state and inverter
shutdown latency. Core's integration test also checks TIM1.BDTR.MOE
and GPIOB.PB0 after fault shutdown. The starter supports mechanical stall
through the existing stall SimInput channel.

This test is an inverter/firmware behavioral proof on a reduced-order BLDC
plant; it does not verify MOSFET transients, actual gate-driver IC behavior,
current-sense FOC or a physical aircraft.

Reproducible public consumer: https://github.com/w1ne/labwired-examples/tree/main/examples/robotics/quadrotor-bldc
