# LabWired HW-building substrate design

Date: 2026-06-11
Status: Approved for implementation
Scope: Make LabWired the state-of-the-art substrate on which agents construct virtual hardware and verify unmodified firmware against it.

## Product boundary

LabWired provides the harness: an agent constructs a virtual board (chip + buses + off-chip components + wiring), runs unmodified firmware against it, and gets deterministic, trustworthy evidence. The agent that designs products on top of this harness is a separate, future product and is out of scope here.

This design closes the gaps between today's substrate and that boundary, sequenced as four independently shippable slices:

1. Component Model IR — agents define new off-chip devices declaratively.
2. Construction surface completion — a validated diagram becomes a runnable board.
3. Run-loop gaps — GDMA peripheral coupling and a virtual network path so real ESP-IDF firmware runs.
4. Proof artifact + public fidelity scorecard.

## Background

Verified competitive findings (2026-06-11 research sweep, sources in `docs/strategy/`):

- Espressif's QEMU fork also ROM-boots the S3 from the real first-stage bootloader, so ROM boot alone is not a differentiator. It has no I2C controller on the S3 target and models zero off-chip devices.
- Renode has the Xtensa ISA but no ESP32-S3 platform; Antmicro has stated plans for more ESP platforms — the window is open but not guaranteed.
- Academic SOTA (Fuzzware, FlexEmu) auto-generates approximate peripheral models for fuzzing, all on ARM Cortex-M; FlexEmu's 9 behavior primitives pass 98.5% of the P2IM fidelity unit tests. None of this work targets Xtensa or off-chip devices.
- The Rehosting SoK's unsolved obstacle list includes fidelity quantification against physical hardware — which LabWired's silicon-validation/HIL methodology already does, and no competitor publishes.

Conclusions baked into this design: agent-self-serve device modeling is where automation will compete, so LabWired needs a declarative model format before someone ports the FlexEmu approach to Xtensa; and the fidelity evidence should be published as a scorecard, not left internal.

## Slice 1 — Component Model IR

### What

Extend `core/crates/ir` with `IrComponent`: a declarative, serializable spec for off-chip devices, sibling to the existing chip-level `IrDevice`. An `IrComponent` declares:

- Identity: part number, vendor, datasheet revision the behavior was derived from.
- Interfaces: I2C (address or address range + address-select pins), SPI, GPIO pins, analog outputs. I2C ships first; SPI/GPIO follow the same shape.
- Register map: reuse the existing `IrPeripheral` register/field structures.
- Behavior primitives, FlexEmu-validated shape: field state, write-to-state transitions, read sources (constant, state, expression over state), timers/events, and output bindings that map internal state to named observables (e.g. LEDn_ON/OFF registers → PWM duty → `servo_angle[n]`).
- Reset/init values and an optional auto-increment pointer rule (the PCA9685/SSD1306 register-pointer pattern).

### Interpreter

A deterministic interpreter in `core` implements the existing `I2cDevice` trait (`core/crates/core/src/peripherals/i2c.rs:22` — address/read/write/start/stop) from an `IrComponent`. Interpretation is a pure state machine over the spec: no clocks of its own, no host time, no randomness. Determinism is gated by the existing VCD-hash methodology.

### Validation gate

- Re-express PCA9685 in IR and assert byte-for-byte equivalence with the hand-written Rust model (`core/crates/core/src/peripherals/components/pca9685.rs`) under the existing replay tests, including the `pcaSetAngle` replay and servo-angle observables.
- Re-express 2–3 simpler parts (BMP280-class register-read sensors) to prove the shape generalizes.
- The Rust models stay; equivalence tests pin the interpreter to them.

### WASM escape hatch

Complex components (displays, modems) stay Rust in-tree for now. This design specifies — but does not implement — the WASM component ABI for a later slice: a sandboxed module with bus transactions in, observable outputs out, fuel-metered execution, no ambient imports (no clock, no I/O), so determinism is preserved by construction. The ABI is recorded here so the IR and diagram schemas reserve the `kind: wasm` component variant from day one.

### Agent surface

- `labwired_define_component`: submit an `IrComponent` (YAML or JSON), receive machine-readable diagnostics in the established `validate_diagram` style (code + message + suggested fix). Accepted components are usable in diagrams by id for the session and persistable to a workspace component library.
- `labwired_list_components` lists built-ins plus defined components, flagging which are IR-backed vs native.

## Slice 2 — Construction surface completion

### Diagram → System Manifest compilation

Today `labwired_validate_diagram` checks parts + wires but a validated diagram is not runnable; agents must hand-author System Manifest YAML. Add compilation: `labwired_compile_diagram` produces a System Manifest from a validated diagram. The manifest stays the single runnable source of truth (run tools keep accepting manifests only); the compiled manifest is returned to the agent so it can be inspected, persisted, and passed to existing run tools unchanged. The agent loop becomes: compose parts + wires → validate → run firmware, with no YAML authoring.

Constructed boards are persistable as named boards (same mechanism as the workspace component library) so an agent can build once and iterate on firmware against it.

### Deeper construction diagnostics

Extend the v0.4 diagnostic codes with checks where real failures live:

- I2C address conflict on a shared bus (two components with the same 7-bit address).
- Interrupt source-ID mapping: peripheral IRQ source must equal the ESP-IDF `ets_isr_source_t` ordinal (the I2C0=42-not-49 class of failure) — checked at compile time, not discovered as a silent never-dispatched ISR.
- Missing I2C pull-up declaration on a bus with open-drain devices.
- Dangling SPI chip-select / unwired required pins.

All diagnostics keep the machine-readable code + hint format.

## Slice 3 — Run-loop gaps

### GDMA peripheral-coupled mode

`core/crates/core/src/peripherals/esp32s3/gdma.rs` supports memory-to-memory only; peripheral-coupled transfers (UART, SPI, I2S) stall. Implement the peripheral coupling so standard ESP-IDF DMA drivers run. Highest-confidence item in this design: the descriptor machinery exists, the work is wiring channels to peripheral FIFOs with correct EOF/interrupt semantics. LCD coupling is explicitly deferred until a target firmware needs it.

### Virtual network path (data only, no RF/PHY)

Goal: unmodified station-mode firmware progresses scan → connect → DHCP → TCP/UDP sockets, with frames bridged to a host-side virtual switch. Implementation builds at the existing `wifi_thunks.rs` boundary; the esp32-open-mac QEMU work is the community reference for what the proprietary driver blob expects from the MAC hardware.

This is the riskiest item in the design and is explicitly timeboxed. Fallback that still ships: a link-up stub mode where WiFi init succeeds and the firmware proceeds without a working data path — sufficient to unblock full-device boot flows whose core function (e.g. dispense path) is not network-dependent. The stub is a documented mode, not a silent degradation: runs report `network: stub` in `result.json`.

No RF, PHY, or radio-timing emulation in any variant.

### Dual-core S3

Verify-and-document only: the flagship firmware already ROM-boots unmodified, so establish what the current dual-core behavior is (APP CPU start, cross-core interrupts, both cores scheduling FreeRTOS tasks), document it in the compatibility matrix, and open targeted follow-ups only if verification finds load-bearing gaps.

## Slice 4 — Proof artifact + public scorecard

### Hero CI test: agent-constructed SpiceDispenser

A scripted MCP session (the same calls an agent would make) constructs the device from parts and verifies the unmodified firmware:

1. `labwired_define_component` for any part not in the library.
2. Build the diagram: ESP32-S3 + PCA9685 on I2C + servos on PWM outputs (+ LED/button as wired on the bench).
3. `labwired_validate_diagram` → `labwired_compile_diagram` → run the unmodified SpiceDispenser firmware.
4. Assert: ROM boot completes, I2C traffic to the PCA9685 matches the dispense sequence, servo-angle observables follow the expected trajectory, and — once the virtual netif lands — the firmware reaches network-connected state. Until then the network assertion runs in stub mode.

This test is the integration gate for Slices 1–3 and runs in CI as a golden-board test (deterministic trace hash).

### Public fidelity scorecard

A published docs page (and CI job that regenerates it) with:

- P2IM's 66 hardware-interaction unit tests on the Cortex-M tier — the benchmark FlexEmu/Fuzzware report against; no emulator vendor publishes its own score.
- Performance: instructions/sec on the comparative-benchmark firmware vs Espressif QEMU on the same host.
- The existing silicon-validation evidence (H563/F103/nRF52 probe results) summarized with links.

Scores are published as measured, including failures — credibility comes from the methodology, not a perfect number.

## Error handling

- IR validation failures, diagram diagnostics, and compile errors all use the established machine-readable code + message + suggested-fix format; no free-text-only errors on agent surfaces.
- The IR interpreter rejects specs it cannot execute deterministically at definition time (unknown primitive, unbounded expression), never at run time.
- Network stub mode is reported explicitly in `result.json`; assertions can require `network: real` so CI cannot silently pass on the stub.

## Testing

- Slice 1: PCA9685 IR/Rust byte-equivalence under existing replay tests; sensor-model equivalence; interpreter determinism under VCD hash; `define_component` diagnostic tests on malformed specs.
- Slice 2: compile-round-trip tests (diagram → manifest → identical run vs hand-written manifest for existing boards); one test per new diagnostic code, including the IRQ-ordinal case.
- Slice 3: GDMA-coupled UART/SPI/I2S driver firmwares as CI fixtures; netif loopback test (firmware echo server reachable from host switch); stub-mode reporting test.
- Slice 4: the hero test itself, plus scorecard regeneration in CI.
- Full verification per repo convention: core fmt/clippy/build/integration suites, `packages/api` and `packages/mcp` test + build.

## Non-goals

- The agent product on top of the substrate.
- WiFi RF/PHY or radio-timing fidelity.
- LLM/SVD auto-generation of IrComponent specs from datasheets (natural next workstream once the IR exists).
- MMU/cache modeling, RISC-V maturation, LCD GDMA coupling (until a target firmware needs it).
- Reducing or rewriting the existing MCP tool behavior (covered by the 2026-06-11 MCP quality design).
