# External mechanical shaft coupling

This interface couples one MCU to one synchronous mechanical solver. It does not establish physical calibration or closed-loop stability.

## Ownership

A bridge DC motor may declare `shaft_owner: external`. The omitted/default value is `native`; existing native electrical and shaft integration remains unchanged. Circuit-armature DC motors and BLDC motors do not support external shaft ownership in this slice.

With external ownership, the native plant integrates winding current from the real H-bridge command and held external velocity. Back EMF remains `Ke * angular_velocity_rad_s`; electromagnetic torque remains `Kt * current_a`. Only the external solver advances shaft position and velocity. Native service never integrates or stalls that shaft a second time.

An externally owned configuration must set all native mechanical parameters to zero:

| Native field | External solver responsibility |
| --- | --- |
| `rotor_inertia_kg_m2` | Complete shaft-equivalent inertia, including any explicitly transferred reflected rotor/gear inertia |
| `viscous_friction_nm_per_rad_s` | Viscous loss, for example the bound joint's damping |
| `coulomb_friction_nm` | Dry friction, for example the bound joint's friction loss |
| `load_torque_nm` | Applied mechanical load |

Zeroing these fields is an explicit ownership transfer, not a calibration operation. A host must first transfer the declared values to its external model or supply an explicit complete replacement. It must not discard them or infer a gearbox. Dynamic native load, drag and stall writes are refused for external shafts. The external motor's `simulation_clock_hz` must equal the MCU's actual declared `cpu_hz`.

## WASM interface

Native `Machine::advance` also refuses an unprimed external model before any supply or CPU mutation, so an ordinary CLI run cannot silently hold an external shaft without a coordinator. Its existing general cycle-budget semantics are otherwise unchanged. A future native coordinator must enforce equivalent audited frontier limits; the strict bound described below belongs to the reviewed WASM interface.

The methods belong to `WasmSimulator`. Multi-node `WasmWorld` construction rejects external shaft models before constructing any node.

### Prime or commit a frame

```ts
sim.apply_external_mechanical_frame({
  at_cycle: 0,
  shafts: [
    { id: "left", position_rad: 0, angular_velocity_rad_s: 0 },
    { id: "right", position_rad: 0, angular_velocity_rad_s: 0 }
  ],
  inputs: [
    { component: "imu", channel: "ax", value: 0 },
    { component: "imu", channel: "ay", value: 0 },
    { component: "imu", channel: "az", value: 1 },
    { component: "imu", channel: "gx", value: 0 },
    { component: "imu", channel: "gy", value: 0 },
    { component: "imu", channel: "gz", value: 0 }
  ]
});
```

The first frame must be at cycle zero. Every frame contains each externally owned shaft exactly once. Its cycle must equal the current MCU cycle and strictly exceed the last committed frame. The priming frame binds the canonical sensor `(component, channel)` set; subsequent frames must contain the same complete set. Ordinary `set_input` and `set_inputs` calls cannot write those owned channels, including through a bus-name alias or an omitted component.

Sensor units remain the existing SimInput channel units. For an MPU6050 these are g and degrees/second; a mechanical adapter must convert from its solver's frame and units explicitly. The native API does not infer sensor orientation, wheel polarity, gearing, gravity compensation or units.

Unknown, duplicate, wrong-owner, missing, stale, nonfinite, out-of-range and unsupported input frames fail before sensor, shaft or torque-interval mutation. All inputs receive strict non-mutating per-device preflight in their exact application order. Stateful inputs without that guarantee are refused. Declarative I2C/SPI/analog inputs preflight exact derived expressions for every intermediate prefix; an I2C device combining exact derived expressions with stateful input rules is currently refused.

A successful frame services the preceding electrical interval, commits all inputs and q/qd without executing firmware between writes, refreshes encoder pins from the supplied angle, then resets the torque interval. Encoder output is a boundary sample. The host must choose a coupling rate that resolves the encoder transitions it needs; this API does not synthesize skipped pulses.

### Advance to a strict cycle frontier

```ts
const interval = sim.step_external_mechanical_to(targetCycle, maximumFuel);
```

`maximumFuel` bounds native work per call; it is not elapsed time. A partial result retains the same interval start. Repeated calls may use the same target until `interval.at_cycle === targetCycle`. The host advances its mechanical solver once at the agreed physical step boundary, using the returned elapsed duration and average torque, then commits feedback at that exact cycle.

The method refuses configurations that cannot guarantee the bound before running an instruction:

- A CPU whose multi-cycle instruction timing can overrun the planner's cycle limit.
- A peripheral without an audited zero additional tick-cycle-cost capability.
- Browser/native JIT execution.
- Another attached co-simulation session.

The ESP32 interpreter path retains its real secondary-core scheduling and the existing legacy single-core IPI/bootstrap service. Native scheduler and peripheral cost semantics are unchanged. Ordinary stepping APIs refuse externally owned shafts, so firmware cannot accidentally run independently of the coordinator.

Cycle and clock fields must fit a JavaScript safe integer. No wall clock, animation frame or UI snapshot is used as the mechanical time base.

### Observe accumulated torque

Both the bounded step and `get_external_mechanical_interval()` return:

```ts
{
  from_cycle: number,
  at_cycle: number,
  clock_hz: number,
  elapsed_s: number,
  shafts: [{
    id: string,
    position_rad: number,
    angular_velocity_rad_s: number,
    current_a: number,
    torque_impulse_nm_s: number,
    torque_average_nm: number | null
  }]
}
```

`elapsed_s = (at_cycle - from_cycle) / clock_hz`. The impulse sums electromagnetic torque times duration over every native electrical service substep, including different bridge/PWM phases. Average torque is impulse divided by elapsed duration, not the last instantaneous torque sample. A zero-duration interval reports a null average.

Observations close pending electrical service at the current cycle but do not consume the impulse. Repeated observations at the same frontier are idempotent. Only a valid committed frame starts a new interval. Electrical integration errors latch for the externally owned plant and prevent a later interval from being presented as valid.

These are synchronous calls. The browser worker must own the native/MuJoCo alternation; UI animation timing and saved stimulus replay must not write owned inputs between its steps. Full frame-journal replay includes the new operations. Partial native runtime snapshots are refused because they omit the external solver's state.
