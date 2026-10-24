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
| Startup and baudrate scan | Implemented | [`src/master_port.c`](../src/master_port.c), [`tests/test_master_startup.c`](../tests/test_master_startup.c), [`tests/test_master_tick.c`](../tests/test_master_tick.c) | EstablishCom submachine of 7.3.2.2 / Figure 36: one wake-up, then test messages at COM3, COM2, COM1; T_REN + T_DMT, T_DMT per rate, T_DWU, n_WU = 2 and T_SD with the PHY inactive between sequences, covered by [`tests/test_master_establish.c`](../tests/test_master_establish.c). The physical WURQ pulse stays in the PHY adapter; none of this timing has been measured on silicon. |
| M-sequence handling | Implemented | [`src/master_port.c`](../src/master_port.c), [`src/master_parameters.c`](../src/master_parameters.c), [`tests/test_master_pd.c`](../tests/test_master_pd.c), [`tests/test_master_startup.c`](../tests/test_master_startup.c), [`tests/test_master_parameters.c`](../tests/test_master_parameters.c) | Real-device validation remains open. |
| Cyclic process data | Implemented | [`src/master_port.c`](../src/master_port.c), [`tests/test_master_pd.c`](../tests/test_master_pd.c), [`tests/test_master_public_flow.c`](../tests/test_master_public_flow.c); on-wire PD output echo against real device firmware in `labwired-core` test `world_station_services.rs :: master_services_isdu_pdout_event_ds_all_pass_on_wire` (this repo's `labwired-real-firmware-model` CI job) | Add more black-box coverage for configured PD sizes and invalid user buffers. |
| RX path and retries | Implemented | [`src/master_port.c`](../src/master_port.c), [`tests/test_master_startup.c`](../tests/test_master_startup.c), [`tests/test_master_tick.c`](../tests/test_master_tick.c); real on-wire RX of device responses over a simulated UART wire in `labwired-core` test `world_station_services.rs :: master_services_isdu_pdout_event_ds_all_pass_on_wire` (this repo's `labwired-real-firmware-model` CI job) | Add line-noise and long-running soak tests with a real PHY. |
| ISDU read/write | Partial | [`src/master_isdu.c`](../src/master_isdu.c), [`tests/test_master_isdu.c`](../tests/test_master_isdu.c), [`tests/test_master_isdu_public.c`](../tests/test_master_isdu_public.c), [`tests/test_master_fake_device.c`](../tests/test_master_fake_device.c); on-wire ISDU read (vendor-name index 0x0010 == "LABWIRED"; event-details index 0x001C) and ISDU write against real device firmware in `labwired-core` test `world_station_services.rs :: master_services_isdu_pdout_event_ds_all_pass_on_wire` (this repo's `labwired-real-firmware-model` CI job) | Verify behavior against real devices. |
| Direct Parameter Page 1 | Implemented | [`src/master_parameters.c`](../src/master_parameters.c), [`tests/test_master_parameters.c`](../tests/test_master_parameters.c), [`tests/test_master_isdu.c`](../tests/test_master_isdu.c) | Real-device validation remains open. |
| Startup device validation | Implemented | [`src/master_parameters.c`](../src/master_parameters.c), [`src/master_port.c`](../src/master_port.c), [`tests/test_master_startup.c`](../tests/test_master_startup.c) | Expand validation once automatic negotiation exists. |
| Device identity / inspection level | Partial | [`src/master_parameters.c`](../src/master_parameters.c), [`src/master_port.c`](../src/master_port.c), [`tests/test_master_parameters.c`](../tests/test_master_parameters.c) | VendorID/DeviceID checked under `TYPE_COMP`/`IDENTICAL`; the SerialNumber leg that distinguishes `IDENTICAL` (ISDU index 0x0015) is not yet wired. |
| Diagnostics | Partial | [`include/iolinki_master/master.h`](../include/iolinki_master/master.h), [`src/master_port.c`](../src/master_port.c), [`src/master_isdu.c`](../src/master_isdu.c), [`tests/test_master_pd.c`](../tests/test_master_pd.c), [`tests/test_master_isdu.c`](../tests/test_master_isdu.c) | Add event detail and link-quality metrics. |
| Multi-port controller | Partial | [`src/master_controller.c`](../src/master_controller.c), [`tests/test_master_controller.c`](../tests/test_master_controller.c), [`examples/master_4port_controller_demo.c`](../examples/master_4port_controller_demo.c) | Define scheduler ownership and port-level runtime policy. |
| SIO DI/DQ | Partial | [`src/master_sio.c`](../src/master_sio.c), [`tests/test_master_startup.c`](../tests/test_master_startup.c), [`tests/test_master_sio_public.c`](../tests/test_master_sio_public.c) | Validate SIO and mode transitions against real adapters. |
| Scheduler/timing | Implemented | [`src/master_port.c`](../src/master_port.c), [`src/master_parameters.c`](../src/master_parameters.c), [`src/master_controller.c`](../src/master_controller.c), [`tests/test_master_tick.c`](../tests/test_master_tick.c), [`tests/test_master_controller.c`](../tests/test_master_controller.c), [`tests/test_master_parameters.c`](../tests/test_master_parameters.c) | MasterCycleTime octet decoded to 100us for validation and pacing; response deadline floored at (11 + 10) T_BIT per A.3.5/A.3.6; T_REN/T_DMT/T_DWU/T_SD startup timers and the T_M-sequence test-message window implemented. Validate timing against hardware captures. |
| Master Command channel/addressing | Implemented | [`src/master_parameters.c`](../src/master_parameters.c), [`src/master_port.c`](../src/master_port.c), [`tests/test_master_parameters.c`](../tests/test_master_parameters.c) | R/W + communication-channel + address encode/decode helpers; the operate transition is composed through them. Page/diagnosis channel services build on this next. |
| Events | Partial | [`include/iolinki_master/master.h`](../include/iolinki_master/master.h), [`src/master_isdu.c`](../src/master_isdu.c), [`src/master_port.c`](../src/master_port.c), [`tests/test_master_isdu_public.c`](../tests/test_master_isdu_public.c), [`tests/test_master_fake_device.c`](../tests/test_master_fake_device.c); on-wire event trigger and `iolink_master_read_event_details` returning code 0x8CA0 against real device firmware in `labwired-core` test `world_station_services.rs :: master_services_isdu_pdout_event_ds_all_pass_on_wire` (this repo's `labwired-real-firmware-model` CI job) | Optional dispatch callbacks (rising-edge event-pending notify + per-event handler) added; fully autonomous async event servicing remains. |
| Data Storage | Implemented | [`src/master_isdu.c`](../src/master_isdu.c), [`tests/test_master_isdu_public.c`](../tests/test_master_isdu_public.c), [`tests/test_master_fake_device.c`](../tests/test_master_fake_device.c); on-wire Data Storage write + readback round-trip against real device firmware in `labwired-core` test `world_station_services.rs :: master_services_isdu_pdout_event_ds_all_pass_on_wire` (this repo's `labwired-real-firmware-model` CI job) | Validate Data Storage restore flows against real devices. |
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
- [x] A.1.6 message checksum and A.1.5 reply layout (CKS event/PD flags, no status octet).
- [x] ISDU transport: Table A.13/A.15 requests, Length/ExtLength, CHKPDU, FlowCTRL segmentation.
- [x] Events over the DIAGNOSIS channel: Table 58 event memory, StatusCode ack (Table 59 T8).
- [x] Table A.10 OPERATE M-sequence codes and Table B.6 reserved descriptor rejection.
- [x] Startup T_DMT/T_DWU gating, n_WU default, response deadline from T_BIT, wake restart after retry exhaustion.

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
- `test_master_isdu_wire`
- `test_master_isdu_public`
- `test_master_tick`
- `test_master_controller`
- `test_master_parameters`
- `test_master_public_flow`
- `test_master_sio_public`
- `test_master_public_header`
- `test_master_fake_device`
- `test_master_real_iolinki_device`
- `master_loopback_demo`
- `master_4port_controller_demo`

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

Verified against the V1.1.5 spec text on 2026-09-18. The wire is now
spec-conformant for the master slice (design contract C1..C6 in
`docs/superpowers/specs/2026-09-18-spec-conformant-wire-design.md`):

- **C1 message checksum (A.1.6).** Requests and replies use the spec XOR
  checksum, seeded `0x52` and compressed 8-to-6 bits by equations (A.1). The
  shared helper is `iolink_checksum6` from the `iolinki` device stack; the
  obsolete `iolink_crc6` is gone. The reply to a READ M-sequence is
  `[OD][PD-in] CKS` and the reply to a WRITE M-sequence is `[PD-in] CKS`
  (Figures A.9/A.10/A.13/A.14), with the Event flag in CKS bit 7 and the
  PD-invalid flag in bit 6 (A.1.5); there is no leading status octet and no PD
  toggle bit.
- **C2 M-sequence control octet (A.1.2, Table A.1/A.2).** `MC = RW<<7 |
  channel<<5 | address`; the master addresses Process, Page, Diagnosis and ISDU
  channels, and reads of unimplemented addresses return 0.
- **C3 ISDU transport (7.3.6, A.5, Table 52).** The ISDU octet stream carries
  `I-Service<<4 | Length`, optional ExtLength, index/subindex/data and CHKPDU;
  Length counts every ISDU octet (A.5.3). Requests are segmented over the ISDU
  channel with FlowCTRL START/COUNT/IDLE/ABORT in the MC address; there are no
  invented control bytes inside the stream.
- **C4 diagnosis channel and events (7.3.8, Table 58/59).** Events are read
  from the device event memory over the DIAGNOSIS channel and acknowledged by
  writing the StatusCode at address 0; the Event flag is taken from CKS bit 7.
- **C5 direct parameters and standard indices (Table B.8, Table A.10).** The
  OPERATE M-sequence capability code is derived from OD width and PD lengths per
  Table A.10, reserved ProcessData descriptor combinations (Table B.6) are
  rejected, and shared index constants (e.g. DetailedDeviceStatus 0x0025) come
  from `iolinki/protocol.h`.
- **C6 timing constants (Table 9, Table 42, A.3.5/A.3.6).** Startup honors T_DMT
  (`t_dmt_tbit`, default 32) before the first test message and T_DWU
  (`t_dwu_100us`, default 400 = 40 ms) between wake retries; the default wake
  budget is n_WU = 2; the response deadline is at least one UART frame (11
  T_BIT) plus the maximum device response delay (10 T_BIT) and never the cycle
  period; after RX retry exhaustion the port returns to STARTUP and re-issues a
  wake-up (7.2.2.1) instead of latching ERROR.

The on-wire `labwired-real-firmware-model` CI proves master↔`iolinki`-device
interop. Both stacks now speak the same spec wire; third-party interop still
needs hardware/conformance validation (see the Open rows).

## Architecture Priority

The scheduler/timing model is now explicit (cycle/deadline pacing plus the
C6 startup/recovery timers). The remaining architecture work is real-PHY
adapters and hardware validation, not the protocol wire.

The next proof is a bench: one commercial sensor on this master through an
out-of-tree PHY adapter, with VendorID, DeviceID, and process data emitted by
`iolink_master_write_gateway_line`. Still open:

- No hardware PHY adapter in this repository. `docs/PHY_BOUNDARY.md` is the contract.
- No real-device sensor or actuator matrix.
- No official IO-Link master conformance test.

