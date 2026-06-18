# iolinki-master Roadmap

`iolinki-master` is a standalone IO-Link master stack. It stays separate from the
device-oriented `iolinki` repository and reuses only the narrow common helpers
needed for frames, CRC, PHY contracts, and shared IO-Link constants.

The goal is not a demo harness. The goal is a portable embedded master library
with clean boundaries:

- no heap allocation in the public API
- caller-owned master/controller storage
- public API types that hide private state
- no coupling to the device-stack singleton API
- hardware-independent protocol core
- explicit, testable timing and scheduler behavior
- standard/service features layered above the cyclic transport

[`docs/IMPLEMENTATION_STATUS.md`](IMPLEMENTATION_STATUS.md) is the factual
ledger. This roadmap is the build order for turning the current local master
behavior into a real master stack.

## Architecture Layers

### 1. Repository and Shared Helper Boundary

Status: implemented.

The master is a sibling stack, not a fork or mutation of the device stack. It
links only the shared helper sources it needs from the local `iolinki` checkout.

Rules:

- [x] Do not add the full device stack through `add_subdirectory()`.
- [x] Do not depend on device singleton entry points such as `iolink_init()` or
  `iolink_process()`.
- [x] Keep shared helper reuse narrow: frames, CRC, PHY contracts, and protocol
  constants.

### 2. Public API and Private State Boundary

Status: partial.

The important architectural move is complete: public callers allocate opaque
port/controller storage, while private state lives under `src/`.

Still missing:

- [x] named public result codes instead of magic integer returns
- [x] documented return contract for every function
- [x] storage-size rationale and size-budget tests
- [x] more black-box tests that avoid reaching into `src/master_internal.h`

This layer matters because every later feature becomes harder to change once
external users compile against it.

### 3. Protocol Core

Status: partial.

Current code can drive the basic startup path, exchange configured cyclic
process data, handle local RX/send errors, and run ISDU transactions in tests.
That is useful, but it is not yet a complete standard-facing master.

Implemented locally:

- [x] startup state progression
- [x] fixed baudrate and auto-baudrate scan
- [x] configured Type 0 and Type 1/2 frame handling
- [x] cyclic PD in/out
- [x] RX accumulation, checksum handling, and retry counters
- [x] ISDU read/write transfer over OD bytes
- [x] fake-device ISDU object-dictionary read coverage
- [x] fake-device Type 0 startup device-validation coverage
- [x] multi-object fake-device ISDU dictionary coverage
- [x] Direct Parameter Page 1 parsing and optional startup validation

Still missing:

- [x] initial capability-driven M-sequence and PD-size selection
- [x] fixed Type 2 selection for code-0 Direct Parameter profiles
- [x] full public M-sequence variant selection coverage
- [x] initial event-code read wrapper
- [x] event detail decode wrapper
- [x] explicit event ack wrapper
- [x] initial Data Storage ISDU read/write wrappers
- [x] Data Storage readback verification wrapper
- [x] initial block-parameterization system-command helpers
- [x] Data Storage parameter-server restore sequencing
- [x] ISDU readback verification helper
- [x] full block parameterization readback sequencing policy
- [ ] protocol behavior validated against real devices

### 4. Timing and Scheduler Core

Status: partial.

This is the biggest architectural gap. Today the stack exposes tick/process and
timeout hooks, but it does not own a precise master-cycle timing contract.
Without this layer, the stack remains a locally testable protocol engine rather
than a serious master runtime.

Required direction:

- [x] define who owns cycle time: caller supplies timer ticks, controller computes next due time
- [x] make `min_cycle_time` affect port-level OPERATE cycle pacing
- [x] represent response deadlines explicitly instead of only accepting a boolean
  timeout flag
- [x] return pending retry status through scheduler-visible timeout ticks
- [x] keep the scheduler hardware-independent
- [x] make port-level cycle pacing testable without wall-clock sleeps
- [x] track response timeout counts in public diagnostics
- [x] track cycle-slip counts in public diagnostics
- [x] track jitter diagnostics
- [x] track derived link-quality diagnostics
- [x] track last event count/code diagnostics from event services

This should be the next major architecture slice after the docs checkpoint.

### 5. PHY Adapter and Hardware Boundary

Status: open.

The protocol core should stay board-agnostic. Real hardware support belongs in
adapter code that implements the PHY contract and supplies timer/line-control
integration without leaking board details into the core.

Required direction:

- [x] define the minimum PHY operations needed for IO-Link, DI, and DQ modes
- [x] decide whether DI needs a new `read_cq_line`-style hook
- [x] separate transceiver control, UART/USART byte transport, and timing source
- [x] add a first fake-device PHY harness before adding board-specific adapters
- [x] add fake-device ISDU object-dictionary read coverage
- [x] add fake-device startup device-validation coverage
- [x] add multi-object fake-device ISDU dictionary coverage
- [x] add fake-device Direct Parameter Page 1 capability profiles
- [x] add fake-device bad-checksum injection coverage
- [x] add fake-device dropped-response timeout coverage
- [x] add fake-device truncated-frame timeout recovery coverage
- [x] expand capability selection into a conformance-style matrix
- [x] keep board support out of `src/master_*.c`

### 6. Controller and Multi-Port Runtime

Status: partial.

The current controller helper initializes and ticks multiple ports. It is not
yet a scheduler, port policy engine, or runtime supervisor.

Required direction:

- [x] fan one controller timestamp out to per-port cycle pacing
- [x] make controller apply response deadlines before issuing another cycle
- [x] support independent port modes and cycle timings
- [x] expose per-port diagnostics without hiding individual port failures
- [x] expose public controller helpers for port count and per-port access
- [x] keep one failed port from corrupting unrelated ports
- [x] allow independent per-port tick events
- [x] add examples for common 1-port and 4-port master usage

### 7. Services Layer

Status: partial.

ISDU and Direct Parameter Page 1 are started. The services layer is where master
features should accumulate above cyclic transport, not inside ad hoc startup
branches.

Required direction:

- [x] keep ISDU state machine independent from startup policy
- [x] add event-code read wrapper
- [x] add event detail decoding
- [x] add explicit event ack wrapper
- [x] add Data Storage service wrappers
- [x] add Data Storage restore sequencing wrapper
- [x] add block parameterization download/upload/store system-command helpers
- [x] add block parameterization readback verification
- [x] add service-level diagnostics and result codes
- [x] expose event service result details in diagnostics

### 8. Hardware Validation and Conformance

Status: open.

Local CTests are necessary but not sufficient. A master stack needs real-device
testing and eventual official conformance validation.

Required direction:

- [x] define a repeatable hardware test matrix
- [ ] run at least one known sensor and one known actuator
- [ ] add long-running timing/error tests
- [ ] compare captured frames against expected IO-Link behavior
- [ ] run official IO-Link master conformance testing when the runtime is mature

## Build Order

### Phase 0: Boundary Lock

Status: complete.

Keep the master independent from the device stack and preserve narrow helper
reuse.

- [x] create separate master repository/build
- [x] compile only shared helper sources from the local device checkout
- [x] keep master public headers separate from device stack internals

### Phase 1: Local Bring-Up Master

Status: mostly complete.

Wake a device, enter preoperate, enter operate, exchange cyclic process data,
and exercise ISDU paths in local tests.

- [x] initialize a master port
- [x] send startup wake-up and Type 0 frames
- [x] enter preoperate and operate
- [x] exchange cyclic process data
- [x] perform ISDU read/write transactions in tests
- [x] expose diagnostics for basic local failures

### Phase 2: Runtime Architecture

Status: next.

Build the missing runtime backbone before piling on more services:

- [x] define public result codes and API contracts
- [x] add event-driven tick dispatch for none, cycle-due, and response-timeout events
- [x] define the full scheduler/timing model
- [x] implement port-level min-cycle-time pacing without wall-clock sleeps in tests
- [x] add public black-box tests for scheduler-visible behavior
- [x] keep the controller/helper boundary explicit for tick event fan-out

### Phase 3: Capability-Driven Master Behavior

Status: open.

Use device information to configure the master instead of relying only on caller
configuration:

- [x] parse Direct Parameter Page 1 capability data
- [x] select compatible M-sequence and PD sizes for currently mapped capability codes
- [x] map code-0 profiles with process data to fixed Type 2 variants
- [x] validate requested configuration against the device capability profile
- [x] fail with named result codes when no compatible mode exists

### Phase 4: Standard Services

Status: open.

Add services above the cyclic transport:

- [x] event read/ack
- [x] Data Storage ISDU read/write wrappers
- [x] Data Storage readback verification wrapper
- [x] Data Storage restore sequencing wrapper
- [x] block parameterization download/upload/store system-command helpers
- [x] parameter readback verification helper
- [x] full block write/readback sequencing policy

### Phase 5: Hardware and Conformance

Status: open.

Add hardware adapters and validate against real devices before calling this a
real master stack.

- [x] define first fake-device PHY harness
- [x] expand fake-device harness into a conformance-style matrix
- [ ] add first hardware PHY adapter
- [ ] test one known sensor
- [ ] test one known actuator
- [ ] run official conformance validation

## Recommended Next Slice

The next implementation slice should be controller-owned cycle scheduling.

Deliverables:

- [x] Add a public time input that can pace port-level cycles without sleeping in tests.
- [x] Replace the boolean-only timeout path in controller-facing code with a model
   that can distinguish "not due", "response timed out", and "cycle due".
- [x] Make `min_cycle_time` affect when the next port-level OPERATE frame may be sent.
- [x] Add tests proving port frames are paced by configured cycle time.
- [x] Add controller time input that lets each port enforce its own cycle deadline.
- [x] Add controller-owned response-deadline scheduling.
- [x] Keep hardware timers outside the protocol core; tests should drive fake time.

The result-code enum remains important, but it is API cleanup. Timing is the
architectural blocker between a protocol test harness and a real master runtime.
