# iolinki-master Implementation Status

This file is the living implementation ledger for the master stack. It should be
updated when a feature graduates from open to partial, or from partial to
implemented.

Status definitions:

- Implemented: code exists and local tests cover the intended behavior.
- Partial: useful code exists, but the standard-facing behavior, API contract,
  or test coverage is incomplete.
- Open: no meaningful implementation yet.

## Status Matrix

| Area | Status | Evidence | Remaining Gap |
| --- | --- | --- | --- |
| Repository boundary | Implemented | [`CMakeLists.txt`](../CMakeLists.txt), [`README.md`](../README.md) | Keep avoiding full device-stack linkage as new shared helpers are needed. |
| Public API shape | Partial | [`include/iolinki_master/master.h`](../include/iolinki_master/master.h), [`tests/test_master_public_header.c`](../tests/test_master_public_header.c), [`tests/test_master_isdu_public.c`](../tests/test_master_isdu_public.c), [`tests/test_master_sio_public.c`](../tests/test_master_sio_public.c) | Add more black-box tests for future service APIs. |
| Opaque storage/private state | Implemented | [`include/iolinki_master/master.h`](../include/iolinki_master/master.h), [`src/master_internal.h`](../src/master_internal.h) | Tune the public storage sizes once the private state stops moving quickly. |
| Port lifecycle | Implemented | [`src/master_port.c`](../src/master_port.c), [`tests/test_master_startup.c`](../tests/test_master_startup.c) | Add a public lifecycle example for downstream users. |
| Startup and baudrate scan | Implemented | [`src/master_port.c`](../src/master_port.c), [`tests/test_master_startup.c`](../tests/test_master_startup.c) | Per-baud wake-up retry (`wake_retry_limit`) added; the physical 80us WURQ pulse and t_WU/t_REN/TDMT timing still live in the PHY adapter and remain unverified on silicon. |
| M-sequence handling | Implemented | [`src/master_port.c`](../src/master_port.c), [`src/master_parameters.c`](../src/master_parameters.c), [`tests/test_master_pd.c`](../tests/test_master_pd.c), [`tests/test_master_startup.c`](../tests/test_master_startup.c), [`tests/test_master_parameters.c`](../tests/test_master_parameters.c) | Real-device validation remains open. |
| Cyclic process data | Implemented | [`src/master_port.c`](../src/master_port.c), [`tests/test_master_pd.c`](../tests/test_master_pd.c), [`tests/test_master_public_flow.c`](../tests/test_master_public_flow.c) | Add more black-box coverage for configured PD sizes and invalid user buffers. |
| RX path and retries | Implemented | [`src/master_port.c`](../src/master_port.c), [`tests/test_master_startup.c`](../tests/test_master_startup.c), [`tests/test_master_tick.c`](../tests/test_master_tick.c) | Add line-noise and long-running soak tests with a real PHY. |
| ISDU read/write | Partial | [`src/master_isdu.c`](../src/master_isdu.c), [`tests/test_master_isdu.c`](../tests/test_master_isdu.c), [`tests/test_master_isdu_public.c`](../tests/test_master_isdu_public.c), [`tests/test_master_fake_device.c`](../tests/test_master_fake_device.c) | Verify behavior against real devices. |
| Direct Parameter Page 1 | Implemented | [`src/master_parameters.c`](../src/master_parameters.c), [`tests/test_master_parameters.c`](../tests/test_master_parameters.c), [`tests/test_master_isdu.c`](../tests/test_master_isdu.c) | Real-device validation remains open. |
| Startup device validation | Implemented | [`src/master_parameters.c`](../src/master_parameters.c), [`src/master_port.c`](../src/master_port.c), [`tests/test_master_startup.c`](../tests/test_master_startup.c) | Expand validation once automatic negotiation exists. |
| Device identity / inspection level | Partial | [`src/master_parameters.c`](../src/master_parameters.c), [`src/master_port.c`](../src/master_port.c), [`tests/test_master_parameters.c`](../tests/test_master_parameters.c) | VendorID/DeviceID checked under `TYPE_COMP`/`IDENTICAL`; the SerialNumber leg that distinguishes `IDENTICAL` (ISDU index 0x0015) is not yet wired. |
| Diagnostics | Partial | [`include/iolinki_master/master.h`](../include/iolinki_master/master.h), [`src/master_port.c`](../src/master_port.c), [`src/master_isdu.c`](../src/master_isdu.c), [`tests/test_master_pd.c`](../tests/test_master_pd.c), [`tests/test_master_isdu.c`](../tests/test_master_isdu.c) | Add event detail and link-quality metrics. |
| Multi-port controller | Partial | [`src/master_controller.c`](../src/master_controller.c), [`tests/test_master_controller.c`](../tests/test_master_controller.c), [`examples/master_4port_controller_demo.c`](../examples/master_4port_controller_demo.c) | Define scheduler ownership and port-level runtime policy. |
| SIO DI/DQ | Partial | [`src/master_sio.c`](../src/master_sio.c), [`tests/test_master_startup.c`](../tests/test_master_startup.c), [`tests/test_master_sio_public.c`](../tests/test_master_sio_public.c) | Validate SIO and mode transitions against real adapters. |
| Scheduler/timing | Partial | [`src/master_port.c`](../src/master_port.c), [`src/master_parameters.c`](../src/master_parameters.c), [`src/master_controller.c`](../src/master_controller.c), [`tests/test_master_tick.c`](../tests/test_master_tick.c), [`tests/test_master_controller.c`](../tests/test_master_controller.c), [`tests/test_master_parameters.c`](../tests/test_master_parameters.c) | MasterCycleTime octet (time-base + multiplier) now decoded to 100us for validation and pacing. Validate timing against hardware captures. |
| Master Command channel/addressing | Implemented | [`src/master_parameters.c`](../src/master_parameters.c), [`src/master_port.c`](../src/master_port.c), [`tests/test_master_parameters.c`](../tests/test_master_parameters.c) | R/W + communication-channel + address encode/decode helpers; the operate transition is composed through them. Page/diagnosis channel services build on this next. |
| Events | Partial | [`include/iolinki_master/master.h`](../include/iolinki_master/master.h), [`src/master_isdu.c`](../src/master_isdu.c), [`src/master_port.c`](../src/master_port.c), [`tests/test_master_isdu_public.c`](../tests/test_master_isdu_public.c), [`tests/test_master_fake_device.c`](../tests/test_master_fake_device.c) | Optional dispatch callbacks (rising-edge event-pending notify + per-event handler) added; fully autonomous async event servicing and real-device validation remain. |
| Data Storage | Implemented | [`src/master_isdu.c`](../src/master_isdu.c), [`tests/test_master_isdu_public.c`](../tests/test_master_isdu_public.c), [`tests/test_master_fake_device.c`](../tests/test_master_fake_device.c) | Validate Data Storage restore flows against real devices. |
| Block parameterization | Implemented | [`src/master_isdu.c`](../src/master_isdu.c), [`tests/test_master_isdu_public.c`](../tests/test_master_isdu_public.c) | Validate block flows against real devices. |
| Hardware PHY adapters | Open | [`include/iolinki_master/master.h`](../include/iolinki_master/master.h) consumes the dependency PHY contract | Add real master-port hardware adapters outside the protocol core. |
| Conformance | Open | Local tests only | Run official IO-Link master conformance testing. |
| Documentation/examples | Partial | [`README.md`](../README.md), [`docs/ROADMAP.md`](ROADMAP.md), [`docs/TESTING.md`](TESTING.md), [`examples/master_loopback_demo.c`](../examples/master_loopback_demo.c), [`examples/master_4port_controller_demo.c`](../examples/master_4port_controller_demo.c) | Add focused examples for ISDU and service workflows as those APIs mature. |

## Checkable Ledger

Use this section for quick progress checks. The table above keeps the evidence
and gap detail.

### Done

- [x] Separate master repository/build from the device stack.
- [x] Compile only narrow shared helper sources from the local `iolinki` checkout.
- [x] Public opaque caller-owned port/controller storage.
- [x] Public named result codes and documented function return contracts.
- [x] Public opaque storage-size rationale and budget checks.
- [x] Private master state under `src/`.
- [x] Port lifecycle states: inactive, startup, preoperate, operate, error.
- [x] Fallible checked mode and baudrate adapter hooks for strict hardware validation.
- [x] Adapter RX flush hook before IO-Link startup and startup baudrate retries.
- [x] Half-duplex TX/RX prepare hooks around core-driven frame sends.
- [x] Startup wake-up, Type 0 idle, transition command, and operate entry.
- [x] Fixed-baudrate startup.
- [x] Auto-baudrate scan across COM3/COM2/COM1.
- [x] Configurable per-baud wake-up retry before scan advance / error.
- [x] MasterCycleTime octet (time-base + multiplier) decode to 100us units.
- [x] Master Command R/W + communication-channel + address encode/decode helpers.
- [x] Rising-edge event-pending dispatch callback.
- [x] Per-event dispatch callback from event details.
- [x] Configured cyclic PD input/output.
- [x] RX accumulation, checksum handling, and retry tracking.
- [x] ISDU read/write transfer in local tests.
- [x] Data Storage ISDU read/write wrappers.
- [x] Data Storage readback verification wrapper.
- [x] Data Storage restore sequencing wrapper.
- [x] Event-code ISDU read wrapper.
- [x] Detailed Device Status event-detail decode wrapper.
- [x] Explicit event ack wrapper.
- [x] Block parameterization download/upload/store system-command helpers.
- [x] Block parameterization write sequencing with readback verification.
- [x] ISDU readback verification helper.
- [x] Detailed Device Status read wrapper.
- [x] Direct Parameter Page 1 parse/apply/get/validate.
- [x] Initial capability-driven config selection from Direct Parameter Page 1.
- [x] Fixed Type 2 capability selection for code-0 Direct Parameter profiles.
- [x] Public requested-config validation against Direct Parameter Page 1.
- [x] Optional startup device-info validation.
- [x] Device identity (VendorID/DeviceID) check with `NO_CHECK`/`TYPE_COMP`/`IDENTICAL` inspection levels.
- [x] Basic diagnostics API.
- [x] Response timeout counter in public diagnostics.
- [x] Cycle-slip counter in public diagnostics.
- [x] Last/max cycle-jitter diagnostics in 100us units.
- [x] Derived link-quality percentage in public diagnostics.
- [x] Last service-level result code in public diagnostics.
- [x] Last event count/code diagnostics from event services.
- [x] Last ISDU service error in public diagnostics.
- [x] Multi-port controller init/tick helper.
- [x] Event-driven tick dispatch for none, cycle-due, and response-timeout events.
- [x] Scheduler-visible pending retry result for response-timeout ticks.
- [x] Separate configured response timeout from min-cycle pacing.
- [x] Port-level `min_cycle_time` pacing with fake monotonic 100us ticks.
- [x] Public scheduler-visible timing snapshot API.
- [x] Per-port controller tick events.
- [x] Controller time-aware tick fan-out for per-port cycle pacing.
- [x] Controller-owned response-deadline timeout scheduling across ports.
- [x] Public controller port-count and port-access helpers.
- [x] 1-port loopback and 4-port mixed-controller runnable examples.
- [x] SIO DQ output through `set_cq_line`.
- [x] SIO DI input through configured `read_cq_line`.
- [x] SIO DI checked C/Q reader for strict hardware validation.
- [x] Dynamic SIO/IO-Link/deactivated mode transitions.
- [x] Public header compile test.
- [x] Public black-box startup/process-data flow test.
- [x] Public black-box ISDU read flow test.
- [x] Fake-device harness for startup, transition, cyclic PD, and port pacing.
- [x] Fake-device ISDU object-dictionary read path.
- [x] Fake-device Type 0 startup device-validation path.
- [x] Capability-profile fake-device Direct Parameter Page 1 helper.
- [x] Multi-object fake-device ISDU dictionary.
- [x] Fake-device event-pending OD status injection.
- [x] Fake-device event-detail ISDU injection.
- [x] Fake-device event ack/code read path.
- [x] Fake-device ISDU write/readback path.
- [x] Fake-device Data Storage write/readback verification path.
- [x] Fake-device bad-checksum injection path.
- [x] Fake-device dropped-response timeout injection path.
- [x] Fake-device truncated-frame timeout recovery path.

### In Progress

- [x] Complete public M-sequence variant selection coverage.
- [x] Add link-quality metrics to diagnostics.
- [x] Clear multi-port runtime policy with controller-computed next due time.

### Not Started

- [x] Full scheduler/timing model.
- [x] Broad capability-matrix selection tests.
- [x] Capability-driven M-sequence and PD-size selection for currently mapped codes.
- [x] Requested configuration validation against device capability profile.
- [x] DI input API/PHY support.
- [x] Dynamic SIO/IO-Link mode transitions.
- [x] Data Storage parameter-server restore sequencing.
- [x] Full block parameterization readback sequencing policy.
- [x] Expand fake-device harness into a conformance-style matrix.
- [x] Define PHY adapter boundary and hardware validation matrix.
- [ ] Real hardware PHY adapter.
- [ ] Real-device sensor/actuator test matrix.
- [ ] Official IO-Link master conformance validation.

## Current Test Targets

Local CTest currently exercises these targets when CMocka is available:

- `test_master_startup`
- `test_master_pd`
- `test_master_isdu`
- `test_master_tick`
- `test_master_controller`
- `test_master_parameters`
- `test_master_public_flow`
- `test_master_sio_public`
- `test_master_public_header`
- `master_loopback_demo`
- `master_4port_controller_demo`
- `test_master_fake_device`

Use this verification loop before committing master-stack changes:

```sh
cmake -S . -B build
cmake --build build
ctest --test-dir build --output-on-failure
git diff --check
```

## Documentation Rules

Update this file in the same commit as implementation changes when the status of
a feature changes. Keep the gap column honest: passing local tests does not mean
hardware or conformance coverage exists.

## Spec Conformance Audit (Interface & System Spec V1.1.5)

Verified against the V1.1.5 spec text on 2026-07-04. Bit-level field encodings
are conformant: MinCycleTime octet (Table B.3), Direct Parameter Page 1 layout
(Table B.1), M-sequenceCapability bits (Figure B.3), RevisionID (Figure B.4), and
the M-sequence control octet — R/W, communication channel, address (Figure A.1,
Tables A.1/A.2).

**Fixed:** ProcessData descriptor decode now isolates Length to bits 0-4 per
Table B.6 (previously the SIO bit corrupted the length, and sub-byte bit lengths
were truncated).

**Known deviations (co-designed with the `iolinki` device stack; a third-party
conformant device would reject them). Fixing requires a coordinated master+device
change and will break the on-wire model until both land:**

- **Startup / OPERATE transition.** The spec's startup state machine requires the
  first message to be `MC = 0xA2` (read MinCycleTime at address 0x02 on the page
  channel) and the OPERATE transition to be MasterCommand `0x99` "DeviceOperate"
  (Table B.2) written to address 0x00 on the page channel. The stack instead
  sends a bare `0x00` probe and a bare `0x0F` transition octet.
- **ISDU I-Service nibble.** Table A.12 defines Read = `0x9/0xA/0xB` and
  Write = `0x1/0x2/0x3`. The shared `IOLINK_ISDU_SERVICE_READ 0x08` /
  `_WRITE 0x09` constants emit `0x8` (reserved) for reads and `0x9` (a *read*
  code) for writes.

The on-wire `labwired-real-firmware-model` CI proves master↔`iolinki`-device
interop, not spec conformance: the device mirrors these same conventions.

## Architecture Priority

Do not treat all open rows as equal. The scheduler/timing row is the current
architecture blocker: without an explicit cycle/deadline model, the master is a
protocol engine that can be driven by tests, not yet a complete embedded master
runtime.
