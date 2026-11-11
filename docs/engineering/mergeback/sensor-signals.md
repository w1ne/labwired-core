# Sensor mergeback: retained outputs and autonomous data-ready

Date: 2026-11-11 (Europe/Budapest). **Generic and board layers active on the review branch. Software acceptance passed; live recapture is tracked in [#1827](https://github.com/w1ne/labwired-core/issues/1827).**

## Decision

The generic resolver, autonomous I2C data-ready engine and bounded LSM303AGR
descriptors were published in PR #1825 at `2dbbcc4ed9c2aadcfb37411d9c437287f6a9e965`.
The owner acknowledged the exact candidate digest for both affected Nordic
records on 2026-11-11. This branch now activates the GPIOTE correction, micro:bit
sensor IRQ placement and source-built board guests as a separate commit.

Original integration base: `75700134a3b31124bb46eae118acc67d65b4b546`.
Board activation base: `2dbbcc4ed9c2aadcfb37411d9c437287f6a9e965`.
The [board-layer patch](microbit-sensor-board.patch) remains an exact reproduction
artifact for that generic base. It is already applied on this branch. SHA256:

```text
68b6b82e3cf882be9e8474fc9852e20c7821816d9408599eb7c826405ea5054d
```

The original [pre-activation validation receipt](sensor-signals.validation.json)
records commands, counts, source hashes, firmware hashes and the original gate
snapshot. Its pending-path labels describe the original generic-only branch; its
failing gate values evaluate the qualified board candidate before owner acknowledgement. The [activation receipt](microbit-sensor-activation.validation.json)
records the exact source match and current owner-approved gate evaluations.
No functional source changed from the qualified candidate during activation.

## Provenance and scope

The public source is
[CrispStrobe/labwired-core at d543a46bba894f9e7e3aa31cdba6feb561b2d08e](https://github.com/CrispStrobe/labwired-core/tree/d543a46bba894f9e7e3aa31cdba6feb561b2d08e).
The port extracts relevant final behavior and preserves its MIT attribution.
The [first mergeback PR](https://github.com/w1ne/labwired-core/pull/1824)
contains the [commit-to-file extraction map](https://github.com/w1ne/labwired-core/blob/c9964663db1ee0b2979e3488627433a9eabadf7d/docs/engineering/fork-port-map-2026-11-10.md).

| Slice | Source anchors | Candidate behavior |
| --- | --- | --- |
| Retained open-drain outputs | [ac659c89](https://github.com/CrispStrobe/labwired-core/commit/ac659c8957bdca026fb9e0b58537a98df28aee3d), [7cf71c2f](https://github.com/CrispStrobe/labwired-core/commit/7cf71c2f05ae747c971ad411c71237a76d818e0f), [e1421d48](https://github.com/CrispStrobe/labwired-core/commit/e1421d48a038ab05c2a83bd132e92596d8d3c110) | Retain each device/role contribution per destination pad. Explicit pull-up and collector inversion. Reject invalid/mixed bindings. |
| Autonomous completion and cancellation | [91935e5c](https://github.com/CrispStrobe/labwired-core/commit/91935e5ced58f293517b79ced4fd0c43d993a388), [bfc30fb5](https://github.com/CrispStrobe/labwired-core/commit/bfc30fb59216ebf98d2620321f6d3adaf8c519f1), [097527df](https://github.com/CrispStrobe/labwired-core/commit/097527df6359753568d40c5ec997a3a8d3ab12a1) | Named, opt-in data-ready events; deterministic deadline order; cancellation; preserve unsubscribed read-driven behavior. Carry the fork's lifecycle-writer and zero-duration validation. |
| Sensor acknowledgement groups | [e8e760bb](https://github.com/CrispStrobe/labwired-core/commit/e8e760bb1acf9dafa0340759af5561a80f98dfdd), [1870ba54](https://github.com/CrispStrobe/labwired-core/commit/1870ba5412b813d218dbbe1a854838f51c369a7f), [8c820d92](https://github.com/CrispStrobe/labwired-core/commit/8c820d9226d84b799a2b13899ba5796abbe6ca34) | `DataReadyRead`/`clear_on_read_all`, enabled-axis acknowledgement and bounded LSM303AGR YAML behavior. |
| Separate GPIOTE/board/guest patch | [295a01c7](https://github.com/CrispStrobe/labwired-core/commit/295a01c7ef497767d8ba3507f841585e0bd3c171), [8f2d638c](https://github.com/CrispStrobe/labwired-core/commit/8f2d638c309b1865ef75a693b2ba05b2d71becb4), pinned fork guest sources | Use actual bus transitions for first-edge detection. Wire the two sensor collectors to P0.25. Exercise real TWIM/EasyDMA and GPIOTE with source fixtures under `crates/core/tests/fixtures/microbit-drdy`. |

### Extraction fixes on the integration base

- Reject data-ready start-register widths outside 1–4 bytes before
  mask construction, preventing the oversized-shift panic.
- Reject data-ready subscriptions on unsupported transports instead of accepting
  a descriptor whose event cannot be delivered.
- Filter attached I2C/SPI output queues by this placement's bound roles. Preserve
  every ordered bound transition and all roles for standalone device capture.
  This prevents indefinite queue growth for disconnected outputs.
- Fan each role out to every matching destination pad, resolving each retained
  net independently. Duplicate identity rejection is local to the same pad.
- Remove the fork's per-tick retained-level restoration pass. The upstream GPIO
  external-level seam retains levels while its register clock is gated; replaying
  every net each tick adds work without supplying missing state. The feature-off and scheduler-on
  clock-gating regressions pass without replay.

## Silicon-record acknowledgement

The owner explicitly accepted the exact GPIOTE content for `nrf52840` and
`seeed-xiao-nrf52840-sense` on 2026-11-11. Both watched digests were recomputed
from the applied source and match that authorization:

```text
previous_ack_digest = 86b0c81ce18f79f9d18d02eb182f135700e90d8fdcc7fc6c3a53a49097599aa8
accepted_digest     = 219f135fe825720f1a2d73e667837512f1fb1f5cbbc2f46eacf747d7dcfc1106
```

| Record | Current gate | Live recapture |
| --- | --- | --- |
| `nrf52840` | Exact drift acknowledged; non-failing | [#1827](https://github.com/w1ne/labwired-core/issues/1827), due 2026-12-11 |
| `seeed-xiao-nrf52840-sense` | Exact drift acknowledged; non-failing | Same capture and tracking issue |

The existing 30-day policy gives an expiry of 2026-12-11. Expiry alone fails the
gate from 2026-12-12. No explicit expiry override was added. The last live capture
remains 2026-08-09, with its probe and result unchanged. The XIAO record refers
to the same physical silicon evidence; it is not an independent second capture.
No fresh hardware capture was performed during this activation.

[validation/manifest.yaml](../../../validation/manifest.yaml) and
[the validation generator](../../../scripts/generate_validation_status.py)
define this acknowledgement and recapture policy. Only the two existing ack dates
and digests were renewed, with dated comments and the recapture issue. Other
board records, watch coverage and performance baselines remain unchanged.

## Acceptance status

All commands and outcomes below are recorded in the linked validation receipt.

| Check | Result | Evidence or required completion |
| --- | --- | --- |
| Original GPIOTE, final guest ELF | **EXPECTED FAILURE** | The same deterministic guest used by the passing candidate observes the pad falling but reports 0 events instead of 1; Cargo exit 101. |
| Corrected feature-off guests | **PASS: 8 tests** | Interval 1; all 8 pass again after source restoration, using the final deterministic ELFs. |
| Corrected scheduler guests | **PASS: 8 tests, each with intervals 1 and 512** | Logs confirm scheduler enabled; final deterministic firmware hashes recorded below. |
| Generic/config/descriptor regressions | **PASS: 417 tests** | Configuration 205; I2C 96; GPIO 19; SPI 41; rule machine 18; attachment 4; resolver 6; registry 13; pad integration 15. Pad integration also passes with the scheduler enabled. |
| Corrected GPIOTE unit tests | **PASS: 11 tests** | First falling transition, polarity/disabled filtering and existing GPIOTE behavior. |
| Autonomous-completion fault control | **EXPECTED FAILURE** | Disabling only autonomous promotion leaves P0.25 high when the no-poll guest requires low. Original source hash restored. |
| Retained-driver fault control | **EXPECTED FAILURE** | Returning the newest driver level releases P0.25 while another sink still holds low. Original source hash restored. |
| Formatting and Clippy | **PASS** | Full candidate, including the board patch and its guests; recorded in the validation receipt. |
| Publication gates | **Required** | Normal pre-commit, pre-push and PR CI gates apply. PR checks report their current state. |
| Performance | **No speedup claim** | Existing thresholds and baseline files remain unchanged; normal PR performance CI applies. |

The generic guest drives actual TWIM/EasyDMA and samples the resolved GPIO with
GPIOTE. It does not inject pad levels. The autonomous case has no device-register
read between START and its first asserted edge. Sensor guests own TWIM; the host
changes command intent in RAM. Tests cover shared retained low/release, absent
address rejection, cancellation, high-byte acknowledgement, first edge, CPU-reset
retention and cold released state. CPU reset is not an off-chip power reset. This acceptance covers
GPIO/GPIOTE observations while guest firmware runs; CPU sleep and wake require
their separate qualification.

ARM GCC is mandatory; missing tooling fails explicitly. Feature-off uses interval
1; scheduler-on uses 1 and 512. `microbit-board-io-test` cannot qualify feature-off
because it enables the scheduler. The final guest receipts correspond to the fixtures
with explicit `PIN_CNF[25] = 0x0c` pull-up writes and deterministic `.file`
metadata. The scheduler runs and restored feature-off run use identical ELFs.

## Primary-source checks and limits

[ST AN4825 Rev 1, April 2016](https://www.st.com/resource/en/application_note/an4825-ultracompact-highperformance-ecompass-module-based-on-the-lsm303agr-stmicroelectronics.pdf)
provides direct support for both acknowledgement choices: page 26, §4.2.1,
requires the accelerometer high bytes of all enabled channels; page 15, §3.3.3,
clears magnetometer DRDY on any channel's high-byte read. Low-byte and status
reads are not the stated acknowledgement conditions. The all-enabled set does
not prescribe read order. BDU is a separate function described on pages 15 and 27.

[LSM303AGR datasheet Rev 11](https://www.st.com/resource/en/datasheet/lsm303agr.pdf),
page 50, §8.11/Table 47, defines accelerometer `H_LACTIVE` polarity. Page 64,
§8.41–8.42/Tables 98–101, separates bit-0 DRDY output enable from bit-6 magnetic
threshold routing and its `IEA` polarity. The bounded DRDY-only model must not
apply threshold polarity to data-ready.

The [official V2.00 schematic](https://raw.githubusercontent.com/microbit-foundation/microbit-v2-hardware/main/V2.00/MicroBit_V2.0.0_S_schematic.PDF),
page/sheet 2, Motion sensor block, shows U6 through DTC143EET1G stages T5/T7:
collectors join `I2C_INT_INT`, emitters connect to ground. This supports explicit
inversion of native sensor outputs. The [official pin map](https://tech.microbit.org/hardware/schematic/)
assigns the combined signal to P0.25. The [official I2C Notes](https://tech.microbit.org/hardware/i2c/)
require the target MCU internal pull-up, including during sleep.

Keep these limitations explicit:

- The resolver is a permanently biased digital net. Firmware now configures the
  pull-up, but these tests do not prove that omitting it causes the physical
  failure. Floating voltage, analog transistor behavior and contention are out
  of scope. The KL27 participant is absent.
- Conversion timing remains fixed at 10 ms; configured ODR is not decoded.
  Rearming from acknowledgement introduces read-latency drift from silicon's
  periodic grid. Samples remain live; BDU, overrun, FIFO and threshold behavior
  are not modeled.
- Magnetometer qualification is continuous 100 Hz with threshold routing off.
  Single-shot return-to-idle, software reset and precise individual ready-flag
  clearing are not established by this acceptance slice.
- Retaining unread results across idle/axis reconfiguration is an internal
  consistency policy, not demonstrated silicon behavior.
- Interval 512 proves observations and edge counts; it does not establish
  single-cycle interrupt delivery.

## Future observer integration: PR #1727

[PR #1727](https://github.com/w1ne/labwired-core/pull/1727) adds device-output
observers. This pad-only extraction does not claim that compatibility. Adding
observer roles to `bound_output_roles` is insufficient when the H-bridge attaches
after the source: the source's queue filter has already been finalized. Future
integration must finalize delivery membership after all placements, or update
an existing source's membership when an observer binds. Preserve observer-only
bus servicing, define raw-source versus resolved-pad observations, and rerun the
PCF8574-to-TB6612 regression. This is an explicit integration dependency.

## Reproduce the original board qualification

The board layer is already active on this branch. To reproduce its original
software acceptance, use a separate worktree at the generic activation base.
The zero-context patch then adds the same GPIOTE correction, micro:bit collector
placement, source-built guests and feature-off CI selection.

```sh
git worktree add --detach ../labwired-sensor-qualification 2dbbcc4ed9c2aadcfb37411d9c437287f6a9e965
cd ../labwired-sensor-qualification

git apply --check --unidiff-zero docs/engineering/mergeback/microbit-sensor-board.patch
git apply --unidiff-zero docs/engineering/mergeback/microbit-sensor-board.patch

LABWIRED_REQUIRE_SENSOR_FEATURE_OFF=1 cargo test --locked -p labwired-core \
  --no-default-features --test microbit_v2_open_drain_guest \
  --test microbit_v2_mag_drdy_guest --test microbit_v2_sensor_drdy_guest -- --nocapture

LABWIRED_REQUIRE_SENSOR_FEATURE_OFF=0 cargo test --locked -p labwired-core \
  --no-default-features --features event-scheduler --test microbit_v2_open_drain_guest \
  --test microbit_v2_mag_drdy_guest --test microbit_v2_sensor_drdy_guest -- --nocapture
```

These commands replay the original software qualification from its generic base.
The active branch records the current acknowledgement in the activation receipt;
replaying the old base does not create a fresh silicon capture.
