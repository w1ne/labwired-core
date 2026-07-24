# Probe V0 — SWD/JTAG recovery probe with logic capture (risk-first spec)

Status: draft · Date: 2026-07-24 · Owner: andrii

## What this is

An 8-channel, 3.3 V-only SWD/JTAG recovery probe with logic capture, driven by
an Orange Pi Zero 2 (H616) and a GW1N-9 FPGA starter board, controlled by
LabWired over Ethernet. V0 exists to prove one loop end to end:

> Remotely flash a physical STM32 over the FPGA, release reset, observe UART and
> eight GPIO/debug signals, verify expected behaviour, and compare the physical
> run against the same firmware in the LabWired digital twin — within a stated
> timing tolerance.

Target reliability: **100 consecutive jobs without manual intervention**, every
capture carrying an explicit overflow/loss status.

## Why this document is ordered the way it is

An earlier draft of this spec was structured feature-first: it front-loaded the
cheap work (SPI link, a full protocol opcode table, a REST/WS API) and treated
the two things that can actually kill the project as ordinary mid-list stages.

This version is **risk-first**. The whole spec is built around retiring two
risks, and every cheap task is demoted to a *prerequisite gate* for them rather
than a milestone in its own right.

- **R1 — the debug executor.** SWD/JTAG runs over OpenOCD `remote_bitbang` → a
  buffered SPI daemon on the Orange Pi → a hand-rolled bit executor in the FPGA.
  This is unfalsifiable until our own logic capture is proven trustworthy, so it
  cannot be validated early no matter how much surrounding scaffolding exists.
- **R2 — the twin-vs-physical comparison.** A bounded timing delta between the
  simulator and real silicon is meaningless until our *own* capture latency and
  the reset-release jitter budget are characterised against a trusted external
  instrument. The tolerance number is an output of measurement, not an assertion.

Everything below serves R1 and R2.

## The trusted reference: Analog Discovery 2

Risk-first bring-up requires a measurement authority we already trust, from
before the first FPGA capture exists. That authority is a **Digilent Analog
Discovery 2 (AD2)**, which we have on the bench:

- 16-channel logic analyzer, 100 MS/s, 3.3 V logic — the reference capture.
- Digital pattern generator — the *known-good source signal*. One instrument is
  both stimulus and reference, so Stage 0 needs no extra hardware (no separate
  DMA-timed MCU).
- 2-channel analog scope — the electrical yardstick for Stage 5 latency
  characterisation, independent of the FPGA.

**Stated limit, inherited by the whole spec:** the AD2 digital front end is
100 MS/s / ~3.3 V and is not a timing authority above roughly 10 MHz of signal
content. V0 samples at 25/50 MS/s, so this is fine — but the Stage 5 bounded
tolerance inherits the AD2's own resolution as its floor. The tolerance number
must be reported as "≥ AD2 resolution", never tighter.

## Hardware envelope (unchanged from V0 intent)

- **Compute:** Orange Pi Zero 2 (H616), Gigabit Ethernet to LabWired, SPI1 +
  one GPIO IRQ to the FPGA. USB gadget mode is **not** a dependency.
- **Gateware:** GW1N-9 starter board, 50 MHz oscillator, single static
  bitstream, 20-pin J14 header as the entire V0 interface (19 fixed-3.3 V
  signals + ground).
- **Target:** a single known **3.3 V** board — Nucleo-F401RE or STM32F401 Black
  Pill (both already in the LabWired catalog).

### J14 pin allocation (19 signals)

| Function | Pins |
| --- | ---: |
| Orange Pi SPI: SCLK, MOSI, MISO, CS | 4 |
| FPGA IRQ → Orange Pi | 1 |
| SWCLK/TCK | 1 |
| SWDIO/TMS | 1 |
| nRESET | 1 |
| BOOT0 | 1 |
| Target UART TX capture | 1 |
| AUX/marker output | 1 |
| Application logic inputs | 8 |

- SWD mode: 8 free application channels.
- JTAG mode: reuse two application inputs as TDI/TDO → 6 free channels.

### Electrical harness (safety is a Stage 3 gate, not an assumption)

Small perfboard / two-layer adapter, 3.3 V target only:

- 100 Ω series resistors on the eight capture inputs.
- 47–100 Ω on SWCLK, SWDIO and control outputs.
- 1 kΩ on nRESET and BOOT0.
- **Defined pull policy:** no capture input is ever left floating. Each of the 8
  application inputs plus UART TX has a defined idle pull (default: pull-down on
  the adapter unless the target actively drives it). This is a correctness
  requirement, not a nicety — floating inputs generate edge storms that destroy
  transition-mode capture (see Stage 3 pass condition).
- 10-pin Cortex debug connector or labelled 2.54 mm header; several grounds.
- **No target power** from FPGA or Orange Pi; target powered separately.
- All FPGA outputs high-impedance until explicitly enabled (verified in Stage 3,
  not assumed).

Out of scope for V0 electrically: 5 V logic, automotive signals, negative
voltage, multi-voltage, any unverified target.

## Gateware architecture

Single static bitstream:

```
50 MHz oscillator
   ├── 64-bit free-running timestamp
   ├── SPI slave (command FIFO + response FIFO)
   ├── debug executor (JTAG line control, SWDIO direction, reset/boot outputs)
   ├── 16-bit raw sampler (8 app signals + debug/reset/UART signals)
   ├── trigger matrix
   ├── BSRAM circular buffer (~32 KiB to raw capture; remainder to FIFOs/events)
   └── transition/event encoder
```

### Mandatory clock-domain decisions (the original draft omitted these)

Every one of these is a spec requirement, verified against the AD2:

- **Two-flop synchronizers** on all asynchronous external inputs before sampling
  or edge detection. No async signal reaches capture or trigger logic
  unsynchronised.
- **Transition-encoder glitch handling:** an edge is recorded only after the
  synchronised value is stable; single-cycle glitches from a metastable resolve
  must not emit a transition event.
- **Floating-input policy** (see harness): enforced in hardware, proven in
  Stage 2 by deliberately marginal AD2-driven edges.
- **Hi-Z-until-enabled outputs:** proven, not assumed, in Stage 3.

### Capture math (informational)

16 sampled bits, ~32 KiB raw history:

| Sample rate | Approx. 32 KiB history |
| ---: | ---: |
| 25 MS/s | 655 µs |
| 50 MS/s | 328 µs |

For full boot sequences use transition mode, drained continuously into Orange
Pi RAM:

```c
struct EdgeEvent {
    uint32_t timestamp_delta;
    uint16_t changed_mask;
    uint16_t new_state;
};
```

## FPGA ↔ Orange Pi protocol (deliberately under-specified here)

A versioned binary framing exists so the link survives later migration:

```c
struct ProbeHeader {
    uint16_t magic;    // 0x4C57
    uint8_t  version;  // 1
    uint8_t  opcode;
    uint16_t length;
    uint16_t sequence;
};
```

**Design decision:** the full opcode table and the edge-agent REST/WS API are
NOT frozen in this document. Designing the cathedral before the link works was
the failure mode of the previous draft. Only the header framing and the Stage 1
minimal opcode set (below) are fixed now. Each later opcode and endpoint is
pinned down when the stage that needs it starts (Stage 4 for debug ops, Stage 6
for the agent API), and appended to a living `protocol/probe_protocol.h`.

Batching rule (fixed now, because it is architectural): the Orange Pi daemon
accumulates operations and submits blocks to the FPGA. It never issues one SPI
request per SWD clock.

## Stages — each is a gate, not a milestone

No stage begins until the previous gate's **pass-evidence artefact** exists
(a committed capture file, a diff log, or a measured number). Durations are
deliberately omitted; the previous draft's day estimates were fiction,
especially for Stage 4.

### Stage 0 — Trusted-reference harness (blocking prerequisite)

Establish the AD2 as measurement authority before any FPGA capture is believed.

- AD2 pattern generator drives a known digital sequence onto a set of lines.
- The same lines feed both the AD2 logic analyzer and (later) the FPGA sampler.
- **Gate:** the AD2 capture of its own generated pattern is byte-exact against
  the programmed pattern, establishing the reference toolchain (AD2 → export →
  our diff harness) end to end.

Pass evidence: committed reference pattern + AD2 capture + a diff script that
reports agreement within AD2 sampling resolution.

### Stage 1 — SPI register link

Orange Pi ↔ FPGA liveness only. No capture, no protocol beyond the minimal set.

FPGA: SPI slave, read-only device ID + protocol version, LED register,
free-running 64-bit timestamp. Orange Pi: SPI1 enabled, `/dev/spidev*` present,
`probe-cli`, one GPIO as `FPGA_IRQ`.

Minimal opcode set frozen here: `GET_CAPABILITIES`, `GET_STATUS`,
`GET_TIMESTAMP`, an LED write, `FPGA_RESET`.

Pass evidence:

```
$ probe-cli info
device: gw1n9-v0
protocol: 1
clock_hz: 50000000
timestamp: increasing
```

plus a demonstrated IRQ round-trip (FPGA asserts, Orange Pi handles).

### Stage 2 — Capture validated against the AD2 (retires R1 precondition)

The internal-counter self-test proves only that the FPGA can sample its own
register. It proves nothing about real 3.3 V edges, metastability, or the
synchronizers. So V0's capture is validated against **real external edges from
the AD2**, not an internal pattern.

Implement: 16-bit sampling (25 MS/s first, then 50 MS/s), rising/falling-edge
trigger, circular pre-trigger buffer, readback over SPI, sequence number +
overflow flag, VCD export on the Orange Pi.

Pass evidence:

- FPGA capture of an AD2-generated pattern matches the AD2's own capture of the
  same lines, within AD2 resolution — no missing, duplicated, or phantom
  samples.
- Correct pre-/post-trigger position.
- Deterministic across 1,000 runs.
- **Marginal-edge test:** an AD2-driven deliberately slow/marginal edge, and a
  deliberately floating line, produce no invented transitions — proving the
  synchronizer + floating-input policy.
- GTKWave/PulseView opens the VCD correctly.

### Stage 3 — Transition recorder + safe target control

Implement: edge detection on all sampled lines, timestamped transition FIFO,
nRESET control, BOOT0 control, AUX marker, hardware-safe output defaults.

Connect the known 3.3 V STM32 target for the first time.

Pass evidence: arm event recording, assert then release reset, capture reset +
UART TX + GPIO startup activity, store a trace lasting **several seconds with
zero overflow and zero phantom edges** on a pulled input. Outputs verified
Hi-Z until enabled (measured with the AD2, not assumed).

### Stage 4 — SWD executor through the FPGA (retires R1)

OpenOCD `remote_bitbang` → `labwired-rbbd` on Orange Pi (buffered SPI) → GW1N
debug executor → target SWD/JTAG. Start at 100–250 kHz, not multi-MHz.

Because Stage 2 made capture trustworthy, SWCLK/SWDIO are **cross-checked by our
own capture during every step** — a bug in the executor is now distinguishable
from a bug in the target.

Pass evidence, in order: read SWD DP IDCODE → enumerate AP → halt CPU → read
core registers → read target memory → reset under debugger control → program a
small firmware image → all of the above with simultaneous **verified** capture
of SWCLK, SWDIO, reset, UART and application GPIO.

Bit-level remote protocol will be slow. Acceptable for V0; a later iteration
replaces it with block-level SWD/JTAG or a native OpenOCD adapter driver.

### Stage 5 — Latency characterisation → declare the tolerance (retires R2 setup)

Only now is the bounded-delta number defined, because only now can it be
measured. Using the AD2 analog channels as the electrical yardstick:

- Measure the probe's own capture latency and the reset-release → first-observed
  edge jitter budget.
- Measure the same reset-release → first-UART-byte timing electrically with the
  AD2, independent of the FPGA capture path.
- **Write the tolerance as a measured number**, reported as "≥ AD2 resolution",
  with its derivation. This becomes the Stage 6 pass/fail bound.

Pass evidence: a committed characterisation report containing the jitter budget,
the AD2-measured reference timing, and the resulting numeric tolerance for the
`reset-release → first-UART-byte` delta.

### Stage 6 — Edge agent + end-to-end twin comparison (retires R2)

Repo structure:

```
hil/probe-v0/
├── protocol/         probe_protocol.h (living)
├── fpga/             gw1n9-v0 gateware
├── orange-pi/        probe-daemon/ remote-bitbang/ edge-agent/
└── tools/            probe-cli/ capture-to-vcd/
```

Agent API (pinned down here, when first needed):

```
GET  /v1/capabilities
POST /v1/jobs
GET  /v1/jobs/{id}
GET  /v1/jobs/{id}/capture
WS   /v1/events
```

Physical job shape:

```json
{
  "target": "stm32f401",
  "firmware_ref": "sha256:...",
  "capture": { "mode": "transitions",
               "channels": ["reset", "uart_tx", "gpio0", "gpio1"] },
  "sequence": [
    {"set_boot0": 0},
    {"assert_reset": true},
    {"flash": {"transport": "swd"}},
    {"assert_reset": false},
    {"run_ms": 3000}
  ],
  "oracle": { "uart_contains": "READY", "maximum_reset_count": 1 }
}
```

The Orange Pi initiates an **outbound authenticated** connection to LabWired; no
inbound router configuration is required.

Pass evidence: **100 consecutive jobs without manual intervention**, each capture
carrying explicit overflow/loss status, and each job's
`reset-release → first-UART-byte` delta evaluated against the Stage 5 tolerance
as a hard pass/fail.

## First end-to-end demonstration

Firmware (Nucleo-F401RE or Black Pill): print `BOOT`; toggle one GPIO ten times;
initialise another GPIO; print `READY`; optionally a compile-time watchdog-reset
loop.

The full loop:

```
LabWired compiles firmware
   ├── runs it in the digital twin
   └── assigns a physical job to the Orange Pi
          FPGA arms capture → reset asserted → OpenOCD flashes →
          reset released → UART + GPIO + reset recorded →
          physical oracle evaluated → simulated vs physical compared
```

Output: simulation status; physical flash status; UART output; reset count;
GPIO transition timing; raw/transition trace; and the twin-vs-physical
`reset-release → first-UART-byte` delta with pass/fail against the Stage 5
tolerance.

## Explicitly deferred (not in V0)

USB 3; analog scope input; 16 free application channels; 5 V / multi-voltage;
target current measurement; target power supply; CAN/LIN/RS-485; enclosure;
production cables; high-speed SWD/JTAG; custom OpenOCD driver.

## Completion criteria

V0 is complete when LabWired can repeatedly perform the loop in "What this is",
achieving 100 consecutive jobs without manual intervention, every capture
carrying an explicit overflow/loss status, and every job's timing delta judged
against the tolerance derived in Stage 5.
