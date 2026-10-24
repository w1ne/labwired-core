# Changelog

All notable changes to the `iolinki-master` project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.1.0] - 2026-10-17

### Added
- A port emits a one-line gateway record (`iolinki-gw/1`) once the device
  identity is known, so bench and bring-up captures name the device without
  extra probing.

### Changed
- **Establish communication per V1.1.5 7.3.2.2 (Figure 36, Table 44).** One
  wake-up request is followed by a TYPE_0 test message at COM3, COM2 and COM1
  until the device answers; there is no wake-up between rates any more. The
  first test message waits T_REN (Table 10) plus T_DMT, and each further rate
  waits T_DMT in its own bit time (Table 42). With no answer at the last rate the
  next wake-up waits T_DWU; after n_WU + 1 wake-ups the failed sequence is
  reported (`IOLINK_MASTER_ERR_RETRY_LIMIT`, `diagnostics.establish_failures`),
  the PHY goes inactive and a new sequence starts after T_SD (Figure 33). The
  port stays in STARTUP instead of latching ERROR. `wake_retry_limit` now counts
  wake-ups per sequence, not per rate.
- **COMLOST restarts from the wake-up.** Message retry exhaustion (MaxRetry = 2)
  in PREOPERATE and OPERATE re-enters STARTUP (Table 46, Table 44 T9/T14,
  Table 85 T3) instead of latching ERROR.
- `IOLINK_MASTER_PORT_STORAGE_SIZE` grew from 1296 to 1312 bytes for
  `config.t_sd_100us` and `diagnostics.establish_failures`.

### Fixed
- **OPERATE M-sequence reply layout (wire change).** The reply to a READ
  M-sequence is now decoded as `[OD][PD-in] CKS` and the reply to a WRITE
  M-sequence as `[PD-in] CKS` with no OD, matching the device stack and Figures
  A.9/A.10/A.13/A.14. Reply framing and OD routing follow the direction of
  the M-sequence on the wire. Device and master must be upgraded together for
  the multi-octet OPERATE types: mixed old/new pairings frame replies with the
  wrong layout and produce checksum errors. Use with iolinki device stack 3.0.0.
- The master now serializes M-sequence exchanges: a new request is deferred
  while a reply is outstanding, and a response timeout flushes late or partial
  reply bytes. Replies carry direction-dependent lengths (A.9/A.10/A.13/A.14),
  so a pipelined send could otherwise mis-frame or desynchronize the stream;
  the ISDU transport and the diagnosis readout advance on reply receipt and
  rely on this ordering.
- `iolink_master_tick_at` / `iolink_master_controller_tick_at` now time out a
  startup test message after T_M-sequence (A.3.6, 58 T_BIT for TYPE_0, widened
  by `response_timeout_100us`). Before, the timed API never left
  "await response" in STARTUP, so a silent port never retried or scanned.
- An undecodable answer to a test message counts as no answer (Table 46
  AwaitReply_1); it no longer latches ERROR after two checksum errors.
- The OPERATE response deadline uses the established rate, not `baudrate`,
  when `auto_baudrate` is set.
- `iolink_master_get_next_tick_time` reports the end of a startup wait.
- Link the real-device test's file-backed NVM hooks directly into its executable
  so weak platform stubs cannot mask parameter writes. Check Data Storage
  readback after reinitializing the stacks.

### Documented
- Align new master commercial quotes with the Indie/Company product-family
  packages: named independent individual or legal company, two/eight total
  scoped assistance hours, and perpetual royalty-free family rights. Retain
  accepted older grants and archive the previous published offer; the device
  stack remains separately licensed.

## [1.0.0] - 2026-09-25

### Changed
- **BREAKING: spec-conformant wire (IO-Link V1.1.5).** The master now speaks the
  spec message checksum (A.1.6), reply layout (A.1.5: `[PD-in][OD] CKS`, no
  status octet), ISDU transport with Length/ExtLength/CHKPDU and FlowCTRL
  segmentation (7.3.6, A.5, Table 52), events over the DIAGNOSIS channel
  (7.3.8, Table 58/59), Table A.10 OPERATE M-sequence codes, Table B.6
  descriptor validation, and the C6 timing rules. This is wire-incompatible
  with every prior release. `IOLINK_MASTER_PORT_STORAGE_SIZE` grew from 1280 to
  1296 bytes to hold the extra timing state.
- **Magic numbers extracted to named constants** (`src/master_internal.h`): the
  retry budget, wake-up byte, frame buffer size, Direct Parameter Page 1 field
  offsets, MinCycleTime / M-sequenceCapability / ProcessData-descriptor bit
  fields, ISDU framing lengths, Data Storage record and event-entry sizes, and
  the startup micro-sequence steps are now named rather than inline literals.
- **MISRA C:2012 pass**: fixed Rule 17.7 (ignored `memcpy`/`memset` returns now
  `(void)`-cast), Rule 13.4 (assignment-in-`while` → explicit read + `break`),
  and Rule 15.7 (all `if … else if` chains terminated). Remaining accepted
  deviations are recorded in [`docs/MISRA_DEVIATIONS.md`](docs/MISRA_DEVIATIONS.md);
  `check_quality.sh` now also finds the Debian x86_64 cppcheck MISRA addon path.

### Added
- **Zephyr module**: `zephyr/`, a west manifest, and a `native_sim` sample.
- **Startup timing config**: `t_dmt_tbit` (T_DMT, default 32 bit times) and
  `t_dwu_100us` (T_DWU, default 400 = 40 ms); `wake_retry_limit` now defaults to
  the spec n_WU = 2 when 0, and the response deadline is floored at (11 + 10)
  T_BIT per A.3.5/A.3.6.

### Fixed
- **ProcessData descriptor decode** (`master_parameters.c`): isolate the Length
  field to bits 0-4 per Table B.6 so a device that sets the (legal) SIO bit, or
  reports a sub-byte bit length, is decoded to the correct octet count.
- **Retry recovery** (`master_port.c`): after RX retry exhaustion the port
  returns to STARTUP and re-issues a wake-up (7.2.2.1) instead of latching
  ERROR; ERROR stays reserved for PHY failures.
- **Startup probe** (`master_port.c`): the MinCycleTime probe octet is stored in
  `device_info.min_cycle_time` under every inspection level, including
  `NO_CHECK`.
- **Table A.10 OD width**: TYPE_2_1 and TYPE_2_2 carry one On-request Data octet.
- **Event readout**: a cyclic OPERATE reply can no longer complete a diagnosis
  readout that has not been sent.

### Documented
- **Spec conformance audit** against Interface & System Spec V1.1.5 in
  `docs/IMPLEMENTATION_STATUS.md`: the wire, ISDU transport, events, codes and
  timing are now conformant for the master slice (C1..C6); the former
  startup/OPERATE-transition and ISDU I-Service deviations are gone.

### Verified
- `cmake-ctest` and `labwired-real-firmware-model` are green against the
  `iolinki` device stack on `develop`.
- This tag is not an official IO-Link master conformance test. It has not been
  run against a third-party sensor or actuator. The physical wake-up pulse
  stays in the PHY adapter.

## [0.2.0] - 2026-07-04

Release-engineering and documentation parity with the sibling `iolinki` device
stack. No protocol behavior changes beyond one guarded cast.

### Added
- **Security package**: STRIDE [threat model](docs/security/THREAT_MODEL.md)
  aligned to the IO-Link Security Design and Development Guideline (Order No.
  10.512) and an EU Cyber Resilience Act overview ([CRA.md](docs/security/CRA.md)).
- **Per-release SBOMs**: `tools/generate_sbom.py` emits reproducible CycloneDX 1.6
  and SPDX 2.3 documents (self-tested by `tools/test_generate_sbom.py`, gated in
  CI via the `sbom-tools` job) and attaches them to each tagged release.
- **Release automation**: `.github/workflows/release.yml` builds with coverage,
  runs tests, generates notes, and publishes a GitHub Release with SBOMs on `v*`.
- **Local quality gate**: `check_quality.sh` (strict `-Werror -Wpedantic
  -Wconversion -Wshadow` compile, cppcheck, opt-in MISRA C:2012, clang-format,
  Doxygen zero-warning) and `tools/run-cppcheck.sh`.
- **Developer tooling**: `.pre-commit-config.yaml`, `ENABLE_COVERAGE` and
  `IOLINK_MASTER_ENABLE_DOCS` CMake options, and a `Doxyfile`.
- **Documentation**: `docs/ARCHITECTURE.md`, `docs/API.md`, `docs/PORTING.md`,
  `docs/CONTRIBUTING.md`, and `docs/RELEASE_STRATEGY.md`.

### Fixed
- Explicit cast on the ISDU read out-length (`master_isdu.c`) to satisfy
  `-Wconversion`; the value is already bounds-checked, so behavior is unchanged.

## [0.1.0] - 2026-07-04

First tagged release. `iolinki-master` is a portable, heap-free IO-Link **master**
protocol library with a caller-owned public API, validated against a co-designed
simulated device and an on-wire firmware model — **not yet a conformant master for
real hardware** (see [`docs/IMPLEMENTATION_STATUS.md`](docs/IMPLEMENTATION_STATUS.md)
for the honest gap list).

### Added
- **Port lifecycle and startup**: inactive/startup/preoperate/operate/error state
  machine, wake-up + Type 0 idle + operate transition, fixed and auto-baudrate
  (COM3→COM2→COM1) scan, and configurable per-baud wake-up retry.
- **Cyclic process data**: configured PD in/out for M-sequence Types 0, 1_1/1_2/1_V,
  and 2_1/2_2/2_V, with RX accumulation, checksum handling, and retry tracking.
- **ISDU services**: read/write with segmentation, Data Storage read/write/restore
  with readback verification, block parameterization (download/upload/store), and
  ISDU readback verification.
- **Device identity**: Direct Parameter Page 1 parse/apply/validate, capability-driven
  config selection, and `NO_CHECK`/`TYPE_COMP`/`IDENTICAL` inspection levels enforcing
  VendorID/DeviceID at startup.
- **MasterCycleTime decoding**: the time-base/multiplier octet is decoded to 100us
  units for validation and cycle pacing.
- **Master Command helpers**: R/W + communication-channel + address encode/decode.
- **Events**: event-code/detail read and ack, plus optional dispatch callbacks
  (rising-edge event-pending notify and per-event handler).
- **Scheduler/timing**: monotonic 100us cycle pacing, response-deadline scheduling,
  and cycle-slip / jitter / link-quality diagnostics.
- **Multi-port controller**: init/tick fan-out with per-port pacing and diagnostics.
- **SIO DI/DQ** modes and dynamic mode transitions.
- **PHY boundary**: hardware-independent protocol core with a documented PHY contract
  and adapter hooks (checked mode/baudrate, RX flush, half-duplex TX/RX prepare).
- **Tooling and tests**: CMake build against the sibling `iolinki` frame/CRC helpers,
  16 CTest targets (CMocka), a fake-device harness, runnable 1-port and 4-port
  examples, and CI (`cmake-ctest` + the on-wire `labwired-real-firmware-model`).
- **Project baseline**: dual-license model (GPLv3 + commercial), coordinated-disclosure
  `SECURITY.md`, and `.clang-format` / `.editorconfig`.

[Unreleased]: https://github.com/w1ne/iolinki-master/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/w1ne/iolinki-master/compare/v0.2.0...v1.0.0
[0.2.0]: https://github.com/w1ne/iolinki-master/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/w1ne/iolinki-master/releases/tag/v0.1.0
