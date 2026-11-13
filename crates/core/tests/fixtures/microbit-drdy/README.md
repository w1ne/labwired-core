# Source-built micro:bit DRDY acceptance fixtures

These are core regression fixtures. They do not depend on an examples checkout
or a committed ELF. Each test builds its Cortex-M4 assembly with
`arm-none-eabi-gcc`, loads the ELF through `labwired-loader`, and runs the real
nRF52833 TWIM/EasyDMA, GPIO and GPIOTE models. A missing compiler fails the test.

## Provenance

The source behavior comes from
[CrispStrobe/labwired-core at d543a46bba894f9e7e3aa31cdba6feb561b2d08e](https://github.com/CrispStrobe/labwired-core/tree/d543a46bba894f9e7e3aa31cdba6feb561b2d08e).
All fixture source retains the MIT SPDX identifier. Explicit `.file` directives
make ELF symbol metadata deterministic across GCC temporary object names; they
do not change the loadable image.

| Fixture | Fork source and extraction |
| --- | --- |
| `mag-drdy.S` | `examples/microbit-v2/mag-drdy.S`; local include path and P0.25 pull-up initialization changed |
| `motion-drdy.S` | `examples/microbit-v2/motion-drdy.S`; local include path and P0.25 pull-up initialization changed |
| `twim-register.inc` | Register-transfer and bounded-wait routines from `examples/microbit-v2/motion-polled.inc`; sample-polling and initialization routines omitted |
| `shared-gpiote.S` | Default P0.25 observer from `examples/microbit-v2/shared-gpiote.S`; unrelated optional nRF52840 fixture driver omitted; P0.25 pull-up initialized |
| `guest.ld` | `examples/microbit-v2/board-io.ld` |

The first-edge assertion requires the behavior corrected by fork commit
[295a01c7ef497767d8ba3507f841585e0bd3c171](https://github.com/CrispStrobe/labwired-core/commit/295a01c7ef497767d8ba3507f841585e0bd3c171).
It must not be satisfied by priming the input with an artificial edge.

## Test contracts

- `microbit_v2_open_drain_guest` places two generic PCF8574 outputs on P0.25.
  Host writes go through TWIM/EasyDMA; the host never writes the pad. Firmware
  samples the line and counts falling GPIOTE events. It checks held-low state
  when either participant releases, both participant orders, explicit collector
  inversion, first falling edge, an absent I2C address, and cold reconstruction.
- Its autonomous fixture adds a 1,000-us conversion. No device-register read
  occurs between START and the first edge. Cancellation must finish within
  500 us, and acknowledgement must be observed released within 500 us, before
  the next conversion. A ready conversion cannot emit duplicate edges.
- `microbit_v2_mag_drdy_guest` and `microbit_v2_sensor_drdy_guest` use the shipped
  LSM303AGR descriptors and board placement. Firmware owns TWIM and all sensor
  register accesses. Host writes change only RAM command intent. The assertions
  cover route masking, completed samples, high-byte acknowledgement, enabled
  axis groups, a retained shared low, polarity, cancellation and recurrence.
- Every fixture enables the P0.25 internal pull-up. The board model still
  assumes an ideal, permanently biased pulled-up net; these tests do not
  reject firmware that omits pull configuration. The physical requirement
  is documented in the [Foundation I2C notes](https://tech.microbit.org/hardware/i2c/).
- CPU reset retains off-chip output state. A newly constructed SystemBus starts
  every open-drain driver released. These tests do not claim an external-device
  power-reset API or peripheral reset fidelity.
- Observer-only device buses and additional output-pad fanout belong to their
  separate bus integration tests. These guests do not add those APIs.

The helper selects interval 1 without `event-scheduler`, and intervals 1 and 512
with it. Both runs are required. The 512 profile asserts guest observations and
edge counts; it does not claim single-cycle interrupt delivery. Sensor command
receipts allow two peripheral service ticks and a guest sample while retaining
a 1-ms command deadline, shorter than the modeled 10-ms sensor conversion.

## Execution

Work-1 has ARM GCC 13.2.1, Rust 1.95.0 and Cargo 1.95.0. CI needs the
`gcc-arm-none-eabi` package; these assembly fixtures do not need vendor firmware,
newlib or the Rust ARM target. The existing PR workspace and feature-off UARTE
lanes install that compiler. The workspace shards discover these integration
targets automatically with the scheduler enabled. The feature-off step names
all three targets explicitly.

From the repository root, with the integration owner's chosen Cargo target:

```sh
LABWIRED_REQUIRE_SENSOR_FEATURE_OFF=1 \
  cargo test --locked -p labwired-core --no-default-features \
  --test microbit_v2_open_drain_guest \
  --test microbit_v2_mag_drdy_guest \
  --test microbit_v2_sensor_drdy_guest -- --nocapture

LABWIRED_REQUIRE_SENSOR_FEATURE_OFF=0 \
  cargo test --locked -p labwired-core --no-default-features --features event-scheduler \
  --test microbit_v2_open_drain_guest \
  --test microbit_v2_mag_drdy_guest \
  --test microbit_v2_sensor_drdy_guest -- --nocapture
```

Do not use `microbit-board-io-test` for the feature-off run: that feature implies
`event-scheduler`. Each build prints the scheduler profile and ELF SHA-256.
Set `LABWIRED_GUEST_ARTIFACT_DIR` to retain the compiled ELFs as receipts.

For negative controls, use an isolated source copy. Disable autonomous completion
while retaining read-driven readiness: `guest_sees_autonomous_data_ready_without_polling`
must fail. Restore the old GPIOTE input-cache implementation while retaining the
new resolver: `guest_observes_first_gpiote_falling_edge_after_channel_enable` must
fail. Bypass retained resolution with the newest raw driver level: the two-driver
sequence must fail when A releases while B still holds low. Restore each change
before recording the corrected pass. No control needs a baseline or validation
acknowledgement edit.
