# Circuit-driven three-phase BLDC motors

A `bldc-motor` with `armature: circuit` reuses the existing BLDC shaft and
trapezoidal back-EMF model. One analog circuit owns the inverter, winding
resistance and inductance, phase currents, and shared power network. The shaft
consumes one complete solved current vector at each analog boundary. It does
not run the ideal six-gate inverter or integrate the winding a second time.

This mode is useful when firmware drives a real modeled gate-driver/FET
network. It does not by itself qualify a specific motor, propeller, controller,
firmware, or flight behavior.

## Configuration

The existing external-device and typed `motor_models` forms remain equivalent:

```yaml
external_devices:
  - id: M1
    type: bldc-motor
    connection: gpio
    config:
      armature: circuit
      resistance_ohm: 0.35
      inductance_h: 0.00018
      torque_constant_nm_per_a: 0.04
      back_emf_constant_v_per_rad_s: 0.04
      rotor_inertia_kg_m2: 0.0000001
      viscous_friction_nm_per_rad_s: 0
      coulomb_friction_nm: 0
      load_torque_nm: 0
      encoder_cpr: 2048
      pole_pairs: 7
      current_limit_a: 40
```

These numbers are controlled test assumptions. They are not manufacturer
measurements. The model requires positive finite R, L, inertia, and constants.
The torque and back-EMF constants use the same SI **phase-peak** convention and
must match within relative `1e-9`; this preserves electromechanical power
consistency. Do not copy line-to-line RMS or rpm/V constants into these fields
without converting their conventions.

A circuit motor has no fixed `supply_voltage_v`, timer, six gate pins, enable
pin, Hall pins, or synthetic motor/inverter/overcurrent/undervoltage output
pins. A nonzero fixed supply or any of these pins is refused. Optional physical
encoder A/B pins must be paired; an index requires both. Electrical faults
belong in the circuit. Mechanical stall and load torque remain shaft inputs.

`current_limit_a` and `overcurrent_trip_steps` are diagnostic only in this mode.
A threshold violation can latch the existing diagnostic fault. It never clips
the solved current, disconnects a winding, or claims hardware protection.

Omitting `armature`, or setting `armature: bridge`, retains the existing
six-gate/timer/Hall model. Its runtime behavior and numeric snapshot fields
remain compatible. The new canonical device metadata also exposes the
existing `coulomb_friction_nm` parameter with default zero.

## Circuit and route contract

Use three series R/L/EMF branches. Each branch starts at its physical phase
terminal and ends at one floating star node. Current is positive **from the
external terminal toward the star**. For example, phase A uses:

```text
Ra phase_a a_r 0.35
La a_r a_l 0.00018
Vemf_a a_l star dc 0
```

Repeat for B and C with the same star. The EMF source positive terminal faces
the external phase terminal. The circuit generator must use winding R/L
values that agree with the motor configuration; the core does not infer and
compare arbitrary netlist topology.

| Path suffix after `board.motor.M1.` | Direction | Meaning |
| --- | --- | --- |
| `phase_a_back_emf_v` | machine to circuit | A phase voltage source value |
| `phase_b_back_emf_v` | machine to circuit | B phase voltage source value |
| `phase_c_back_emf_v` | machine to circuit | C phase voltage source value |
| `phase_a_current_a` | circuit to machine | Solved A branch current |
| `phase_b_current_a` | circuit to machine | Solved B branch current |
| `phase_c_current_a` | circuit to machine | Solved C branch current |
| `supply_v` | circuit to machine | Optional measured rail, for readback only |

In a shared world, prefix each route with its exact owning node, for example
`node.U2.board.motor.M1.phase_a_current_a`. The same motor ID can exist in
another node without aliasing. A physical shaft has exactly one owner. The
shared circuit can include drivers from several nodes; ownership does not
create a second copy of the winding or power island.

All three current outputs and all three EMF inputs must appear exactly once
in the same in-core `analog` model with positive `step_ns`. Duplicate phases,
incomplete routed vectors, unknown motors, wrong motor kinds, multiple owners,
and standalone sessions with mixed model intervals are refused. Circuit motor
IDs use `[A-Za-z0-9_-]+` so an ID cannot create extra path segments.

## Boundary order and numerical limits

At each interval the circuit samples the previous boundary's back-EMF, holds
it while solving the interval, and returns its endpoint currents. The shaft
uses that vector and the interval-start angle to compute torque, advances
once, and publishes back-EMF for the next interval. The initial time-zero
observation primes currents without advancing mechanics or diagnostic trip
counts.

The shaft still advances when its owning MCU is held in supply or external
reset. CPU motor service skips this circuit mode, so additional instructions
or host call chunking do not advance the shaft a second time. Optional encoder
feedback is published from the resulting shaft state.

The floating-star current sum must satisfy
`abs(Ia + Ib + Ic) <= 1e-9 + 1e-6 * (abs(Ia) + abs(Ib) + abs(Ic))` amperes.
The core refuses invalid/nonfinite vectors. It does not project them onto a
zero-sum vector. Each apply clears its pending vector before collecting the
new observation, so a rejected prior boundary cannot supply a missing phase
on a retry. Boundaries must advance by exactly the declared interval.

This is interval-start zero-order-hold coupling. Gate edges inside an interval
are sampled, not reconstructed. Select an interval against the electrical
and mechanical time scales and demonstrate convergence for the intended
scenario. The generic tests compare 2,000, 1,000, and 500 ns intervals; they do
not establish a universal interval for DShot, sensorless startup, or flight.

## Readback

`MotorSnapshot` and browser motor state retain `current_a: null` for a circuit
BLDC because phase currents do not identify DC battery current. They retain
`bus_voltage_v: null` until an explicit physical `supply_v` probe is routed.
A circuit DC motor also retains an unknown bus voltage until measured. No
zero or nominal supply is inserted to make these quantities look observed.

After the first complete current observation, `phase_currents_a` contains the
three solved currents and `control_state` is `circuit`. Before it, control is
`circuit:no-current` and phase-current observation is absent. Circuit BLDC
snapshots include:

```json
{"coupling":{"mode":"interval-start-zero-order-hold","step_ns":100,"completed_time_ns":300}}
```

Before a boundary is bound/completed, the corresponding timing fields are
null. Existing bridge snapshots omit `coupling` and retain numeric bus/current
values. Native environment results expose nonempty motor vectors under
`node_motors`, keyed by exact node. `WasmWorld.get_actuator_states(node_id)` and
`WasmWorld.motor_series(node_id)` provide the same node-owned shaft observations
and series; they return errors for unknown nodes or serialization failure.
The World actuator getter currently provides motor states.

## Evidence and scope

The executable regression is `crates/core/tests/world_circuit_bldc.rs`. It
constructs two real Cortex-M nodes with small instruction loops and two motors
on one solved supply. It checks independent current/rotation signs with the
same local motor ID, shared-rail sag, reset-independent mechanics, absence of
duplicate integration, complete route ownership, and rejected stale vectors.

The energy case integrates source work, phase/switch/source resistor heat,
inductor energy, and shaft energy. At 2,000/1,000/500 ns the energy residuals
are 1.287899/0.663451/0.336604 microjoules. The finest residual is approximately
0.0845% of the 398.262 microjoules supplied; speed changes from 508.884 to
507.083 rpm between the last two intervals. These are results from the
specified deterministic fixture, not measured hardware agreement.

The separate native ESC receipt in
`docs/evidence/native-esc-short-world-20261112.json` records a motor-free,
constant-16.8 V operating-point run of four candidate AT32 models with both
vendor smoke and unchanged AM32 firmware. It establishes only construction and
10 microseconds of early shared-world execution. It does not establish native
ADC conversion, complete firmware startup, cold power sequencing, motor
commutation, or flight. The candidate hardware source remains subject to its
unchanged human silicon-review gate.
