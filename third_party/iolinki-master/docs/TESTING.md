# iolinki-master Testing

This stack currently has local software tests. It does not yet have real-device
hardware validation or official IO-Link master conformance coverage.

## Test Layers

### Unit and Protocol Tests

Status: implemented.

These tests exercise focused behavior with fake PHY callbacks and explicit
frames:

- startup state changes
- cyclic process data
- ISDU read/write
- direct parameter parsing and validation
- tick/event behavior
- controller fan-out
- public header isolation

Run:

```sh
ctest --test-dir build --output-on-failure
```

### Fake Device Harness

Status: started.

`tests/fake_iolink_device.c` provides a small simulated device behind the master
PHY API. It reacts to master transmissions and queues device responses instead
of making each test manually inject every byte.

Current coverage:

- wake-up detection
- Type 0 startup response
- transition command detection
- cyclic OPERATE response with PD valid
- port-level `min_cycle_time` pacing through `iolink_master_tick_at()`
- Direct Parameter Page 1 capability-profile injection
- bad response checksum injection
- dropped response timeout injection
- truncated-frame timeout recovery

This is the first bridge between unit tests and a real conformance rig. It is
still intentionally small.

### Real Device Stack Harness

Status: implemented for one in-memory port.

`tests/test_master_real_iolinki_device.c` links the real vendored `iolinki`
device stack sources into a test-only library, then connects it to
`iolinki-master` through in-memory PHY queues. This exercises real master
startup and cyclic process-data exchange against the real device stack without
pulling the device singleton into the production master library.

CI runs this through the normal CTest suite. Because the device stack currently
lives under the LabWired repository, GitHub Actions needs a
`LABWIRED_CHECKOUT_TOKEN` secret with read access to `w1ne/labwired`.

### Missing Test Layers

- [x] fake-device Direct Parameter Page 1 capability profiles
- [x] broader capability-matrix tests for M-sequence and PD-size negotiation
- [x] fake-device ISDU object dictionary
- [x] fake-device ISDU write/readback path
- [x] fake-device event-pending OD status injection
- [x] fake-device event-detail injection
- [x] fake-device event ack tests
- [x] fake-device Data Storage behavior
- [x] fake-device bad CRC injection
- [x] fake-device dropped response timeout injection
- [x] fake-device dropped byte/truncated frame injection
- [ ] long-running soak tests
- [ ] real hardware PHY adapter tests
- [ ] real sensor/actuator test matrix
- [ ] official IO-Link master conformance validation

## Current CTest Targets

- `master_loopback_demo`
- `test_master_startup`
- `test_master_pd`
- `test_master_isdu`
- `test_master_tick`
- `test_master_controller`
- `test_master_parameters`
- `test_master_public_flow`
- `test_master_public_header`
- `test_master_fake_device`
- `test_master_real_iolinki_device`

## Hardware Validation

The repeatable hardware matrix is defined in
[`HARDWARE_VALIDATION.md`](HARDWARE_VALIDATION.md). It is a required checklist
for future PHY adapter and real-device runs, not evidence that those runs have
already happened.

## Verification Loop

Use this before committing:

```sh
cmake -S . -B build
cmake --build build
ctest --test-dir build --output-on-failure
git diff --check
```

## Honesty Rule

Passing local tests means the master behavior is locally verified. It does not
mean the stack is hardware-tested, timing-certified, or IO-Link conformance
validated.
