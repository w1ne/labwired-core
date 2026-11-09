# LumenPnP closed-loop verification

This example executes a Cortex-M4 diagnostic application against the Opulo LumenPnP Rev05 motherboard pinout. Six STEP/DIR interfaces drive a dual-nozzle pick-and-place mechanism. Contact from the evaluated kernelCAD geometry controls pneumatic leakage. Pump winding current and shaft speed determine evacuation, and the firmware makes pickup decisions from XGZP6857D registers over the PCA9545A I2C switch.

The mission homes the machine, places a QFN with nozzle A, detects an empty SOIC pocket with nozzle B, retries at a populated pocket, places that component at 90°, and places a chip package with nozzle A at 45°. The `open-x-step` scenario checks a disconnected driver input and homing timeout. The `vacuum-leak` scenario opens nozzle A's seal during board travel and requires firmware pressure fault 61 with disabled outputs.

## Run

Use the repository's pinned Rust toolchain and an ARM GCC toolchain with `arm-none-eabi-gcc`, `arm-none-eabi-size`, and `arm-none-eabi-nm` on `PATH`. The captured firmware was built with ARM GCC 13.2.1. `ARM_TOOLCHAIN_BIN` can specify a compiler directory. `ARM_CC`, `ARM_SIZE`, and `ARM_NM` can select individual executables.

```bash
example_dir=crates/core/examples/lumenpnp
bash "$example_dir/build.sh"
cargo run --release -p labwired-core --features event-scheduler \
  --example lumenpnp_capture -- --base "$example_dir" \
  --scenario healthy --out "$example_dir/healthy"
cargo run --release -p labwired-core --features event-scheduler \
  --example lumenpnp_capture -- --base "$example_dir" \
  --scenario open-x-step --out "$example_dir/open-x-step"
cargo test --release -p labwired-core --features event-scheduler \
  --test lumenpnp_platform_models
```

Every run exits successfully only when its complete acceptance checks pass. A `--seconds` cutoff before completion returns a failing exit status and records the failed terminal-state check. `--coupling-us` accepts 1000, 500 or 250 for step-size comparison.

## Captured evidence

| File | Contents |
| --- | --- |
| `telemetry.csv` | One row per selected coupling boundary, with actual axis position, nozzle tips, pneumatic pressure, firmware pressure readings, pump current, rail voltage, shaft speed, output levels and part state |
| `logic.csv` | Every observed STEP, DIR, EN, pump, valve and limit transition with native cycle timestamp |
| `analog.csv` | Every 100 µs circuit solution for pump winding current, rail and drain voltage |
| `events.jsonl` | Geometry contact, force-dependent attachment, release, placement and drop events |
| `uart.log` | Bytes emitted by the executed firmware on USART1 |
| `verdict.json` | Required checks, source and executable hashes, calibrated axes, raw edge totals, placements and run accounting |
| `analog-series.json` | Change-point current, voltage and motor-speed traces for compact plotting |
| `executed-system.yaml` | The exact model configuration used by that scenario |

`fw_placed_count` is the controller's completed-cycle count. `placed_count` is the independent mechanical result. `attached_a` and `attached_b` contain a source-pocket index or −1. The observation window at `0x20010000` carries firmware state for inspection. All actuation and all feedback use the modeled GPIO and I2C interfaces.

The healthy checks require three physical placements, one failed pickup and one successful retry, correct placement angles, captured rigid-kinematic evidence, all six raw GPIO counters matching driver counts and angles, a matching firmware time base, and complete captured traces. The disconnected-X run requires a static X motor, emitted controller STEP edges, a homing fault and zero placements. Every scenario rejects driver timing violations and sampled dual-Y skew above one microstep (0.025 mm). Healthy operation additionally requires all six drivers to have exercised their timing checks. Dual-Y skew is evaluated at coupling boundaries, and its maximum remains latched through the run.

## Model parameters

| Subsystem | Model and selected parameters |
| --- | --- |
| MCU | STM32F407VE, 512 KiB flash, 128 KiB main SRAM, 168 MHz HCLK |
| Motion scheduler | Hardware SysTick at 10 kHz, 1 ms cubic trajectory planner and pulse distribution |
| Drivers | TMC2226 active-low enable, rising STEP edges and DIR polarity, ideal calibrated pulse-count kinematics |
| Microstep setting | MS1 and MS2 at ground select 1/8 microstepping, giving 1600 pulses per motor revolution |
| Linear transmission | Evaluated 20-tooth GT2 pulley geometry gives 40 mm/revolution and 40 pulses/mm |
| Rotary axes | 1600 pulses/revolution, 0.225°/pulse |
| Contact | Evaluated package bounds, 0.3 mm XY and 0.15 mm Z contact tolerance, CAD mass from 1800 kg/m³ effective package density |
| Suction aperture | CAD nozzle-bore radius of 0.25 mm |
| Pump circuit | 24 V rail, 18 Ω winding, 20 mH inductance, 50 mΩ energized low-side switch and flyback diode |
| Pump rotor | 0.025 N·m/A torque constant, 0.025 V·s/rad back-EMF constant, 2×10⁻⁶ kg·m² inertia, 2×10⁻⁵ N·m·s/rad viscous drag |
| Pneumatic chamber | 10 mL volume, 60 mL/s nominal flow at 700 rad/s, 31,325 Pa ultimate absolute pressure |
| Leak and vent | 420 mL/s open-nozzle conductance, 0.3 mL/s sealed leakage and 400 mL/s vent conductance |
| Pressure load | Up to 5 mN·m load torque, proportional to developed vacuum, returned to the pump rotor |
| Sensor | XGZP6857D200KPGPN33 legacy protocol, address 0x6D, command 0x30, 20 ms conversion and signed 24-bit pressure at 0x06–0x08 divided by 32 |
| Switch | PCA9545A at 0x70, nozzle sensors on channels 0 and 1, channel selection latched at STOP |
| Firmware decision | Pickup below −25 kPa, failed-pick deadline 500 ms, release above −0.25 kPa |

The pump, tube, leakage, switching resistance and material-density values are declared engineering assumptions. The motion model provides ideal kinematics. The contact model uses rigid package bounds and a quasi-static force-versus-weight attachment criterion. The custom diagnostic firmware contains the mission and exercises the fitted interfaces directly.

The chamber integrates a fixed-volume pressure balance with pump flow proportional to shaft speed. Geometry selects sealed or open conductance. Suction force follows the pressure difference multiplied by the CAD aperture area. Attached parts follow the actual nozzle transform. Attachment preserves the measured nozzle-to-part position and angle. Release below the weight threshold resolves a quasi-static board support plane only within the declared vertical tolerance and with the entire rotated package footprint inside the board. Placement also requires the target position and angle. The release event retains the original support gap. Free flight, impact and solder contact require additional models.

## Pin map

| Function | STEP | DIR | EN |
| --- | --- | --- | --- |
| X | PB15 | PB14 | PD9 |
| Y1 | PE15 | PE14 | PB13 |
| Y2 | PD6 | PD7 | PA3 |
| Z | PE7 | PB1 | PE9 |
| Nozzle A | PC4 | PA4 | PB0 |
| Nozzle B | PE11 | PE10 | PE13 |

| Interface | Pins |
| --- | --- |
| Pump A and valve A | PB8 and PB9 |
| Pump B and valve B | PE5 and PE6 |
| X, Y and Z endstops | PC6, PD15 and PD14 |
| I2C1 | PB6 SCL, PB7 SDA |
| Controller-side UART observation | PA9 USART1 TX |

## Geometry provenance

`geometry-contract.json` is the executable dimensional contract from `smt-pick-place.kcad.ts`. It records the evaluated board and package dimensions, nozzle aperture, material-derived package masses and the coupled nozzle transforms. `kernel-geometry-verification.json` records the source hash and solved-mate calibration evidence. Its provenance identifies the saved local capture, and its coverage preserves the failed quality review and unverified physical calibration. The runner requires `kinematics_verified` and rejects mismatched source or receipt bytes before executing. The model uses a fixed machine-frame root and a compact verification work envelope.

The mechanical derivative follows the CERN-OHL-W-2.0 terms and attribution in `LICENSE-Opulo.txt`.

The source reference is the Opulo LumenPnP repository at commit `64cf68d5b7980f095ef11913065e7f45a1e3e447`. The pin map and driver strap settings were read from its original motherboard PCB. [Opulo reference source](https://github.com/opulo-inc/lumenpnp/tree/64cf68d5b7980f095ef11913065e7f45a1e3e447)

The sensor register model follows the manufacturer's V2.5 XGZP6857D protocol, matching the fitted non-C sensor and the product's OpenPnP commands. [XGZP6857D V2.5 datasheet](https://www.endrich.com/Datenbl%C3%A4tter/Sensoren/XGZP6857D-Pressure-Sensor-V2.5.pdf)

## Bounded native builds

A bounded native build can use one compiler job, unwind panic handling, optimization level 1, and debug level 0. The repository retains overflow checks. The exact build settings and source identity belong with each capture.

```bash
LABWIRED_SOURCE_COMMIT="$(git rev-parse HEAD)" \
CARGO_PROFILE_RELEASE_PANIC=unwind \
CARGO_PROFILE_RELEASE_OPT_LEVEL=1 \
CARGO_PROFILE_RELEASE_LTO=false \
CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_BUILD_JOBS=1 \
  cargo build --release -p labwired-core --features event-scheduler \
  --example lumenpnp_capture
cargo test -p labwired-core --features event-scheduler \
  --test lumenpnp_platform_models --test step_dir_timing \
  --test stm32_timer_clock_tree
cargo test -p labwired-core --features event-scheduler --example lumenpnp_capture
```

The build produces `target/release/examples/lumenpnp_capture`. The explicit source revision is embedded in each run's verdict. Capture evidence also includes the executable, runner source, vacuum source, firmware, chip, CAD source, geometry contract and receipt hashes.

## Repair X STEP in the same Machine and guided CAD checkpoints

The repair-x scenario disconnects the existing X driver STEP input while the
MCU continues to emit GPIO edges. After the safe fault 40, it reconnects
that driver and resets the MCU while keeping the stepper position, plant and
mechanical model. Three placements are completed in the same Machine instance.

With --operator-gated and --scenario repair-x, the runner emits a JSONL
MOTION_POSE every 0.5 simulated seconds. It pauses at X_STEP_REPAIR_REQUIRED
and accepts only REPAIR_X_STEP, then pauses with an attached part at
CAD_POSE_CHECKPOINT and accepts only CONTINUE. The operator evaluates each
xyzab checkpoint via the separate KernelCAD MCP service. The native check
external_pose_checkpoint_acknowledged proves an input acknowledgment, not
a cryptographically verified CAD execution receipt.

All four scenarios pass at 1000, 500 and 250 us. A real guided run with
separate KernelCAD evaluations at [20,20,50,0,0] and [50,60,25,0,0]
finished at [290,250,25,45,90], with 32/32 native checks passed.
Full swept-collision verification and physical load qualification remain
outside the simulator's demonstrated scope.
