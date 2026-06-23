# Plan: declarative peripheral authoring

## Why this is the durable advantage

LabWired's edge is silicon fidelity: it boots unmodified firmware and reproduces
what real hardware does, including the behaviour a passing test never exercised
(gated clocks, real memory sizes, status-flag semantics). A single forgotten
clock-enable or an out-of-bounds store is caught instead of silently passing.

That edge is only durable if it *scales*. Today a new chip rides for free on the
SVD ingestor for its **register map**, but the **behaviour** — which bits gate a
peripheral, when a status flag asserts, what a register reads after reset — is
hand-written Rust, dispatched by a `match canonical_type` in
`crates/core/src/bus/from_config.rs`. Adding genuinely new behaviour means adding
a model and a match arm. Breadth is therefore bounded by engineering hours, and
"how many chips, accurately" is the axis the whole strategy rests on.

The goal of this plan: make each new chip's *fidelity* cheap to author, so
coverage grows without a linear growth in hand-written models. The register map
already comes from SVD; what is missing is a declarative way to express the
**behaviour** that fidelity depends on.

## KPI

The metric that matters is **time-to-faithful-peripheral**: from datasheet to a
model that passes the fidelity benchmark (`examples/f103-fidelity-bench`) for that
peripheral. Every phase below is judged by whether it lowers that number while
the benchmark score stays at full marks. Track it explicitly; a capability that
does not move this KPI is out of scope.

## What's hard (and what isn't)

- **Register maps: solved.** The SVD ingestor already emits `PeripheralDescriptor`
  YAML with addresses, fields, and reset values.
- **Behaviour: the actual cost.** The high-value behaviours are small and
  regular, and today they live in code:
  - *clock gating* — a peripheral is inert until an enable bit in a clock
    register is set (already modelled via the chip-YAML `clock:` field — the
    proof that a declarative slice works).
  - *status-flag semantics* — e.g. a "transmit ready" flag that reads as set only
    when the peripheral is enabled and clocked.
  - *reset / read-only / write-1-to-clear* register semantics.
  - *simple side-effects* — a write to one register updates the readback of
    another (an output latch reflected in its readback register).
- **Genuinely complex peripherals stay in Rust.** DMA engines, CAN/FDCAN, USB,
  crypto: these have real state machines and should remain hand-written. The plan
  is not "no more Rust"; it is "stop hand-writing the regular 80%".

## Approach: a register-semantics layer over the ingested map

Extend the descriptor the SVD ingestor already produces with an optional,
declarative **behaviour** section, and add one generic peripheral that interprets
it. Authoring a regular peripheral becomes: ingest the SVD (free map) + write a
short behaviour spec. No new Rust, no new match arm.

A sketch of the behaviour vocabulary (condition → action over named
register/fields, which is enough to express the bullets above):

```yaml
# attached to an ingested peripheral descriptor
behavior:
  gated_by: { reg: apb2enr, bit: 14 }      # inert unless this bit is set
  fields:
    SR.TXE:   { reads: 1, when: enabled }  # status flag semantics
    CR1.UE:   { access: rw }
    DR:       { on_write: tx_byte }         # named side-effect into the bus
  reset: { CR1: 0x0000, SR: 0x00C0 }
```

The generic interpreter enforces `gated_by` against the clock model already in
the bus, computes flag reads from declared conditions, and routes named
side-effects (`tx_byte`, `rx_byte`, `irq`) to existing bus primitives. The
existing `match` arms remain for the complex peripherals; the default arm becomes
"interpret the behaviour spec".

## Phases

1. **Spec + generic interpreter (slice).** Define the behaviour schema in
   `crates/config`, add a generic interpreted peripheral in `crates/core`, and
   reproduce *one* existing hand-written model (the F1 UART) purely from a
   behaviour spec. Done when `examples/f103-fidelity-bench` passes with the UART
   served by the interpreter instead of `uart.rs`. This proves the vocabulary is
   sufficient before broadening it.

2. **Ingestor emits behaviour stubs.** Teach the SVD ingestor to pre-fill the
   behaviour section it *can* infer (reset values, access types, obvious enable
   bits from the clock tree), leaving only the semantic gaps for a human or an
   agent to fill. Measure the KPI before/after.

3. **Authoring loop + fidelity gate.** Wire a new peripheral through:
   ingest → fill behaviour → run the fidelity benchmark for that peripheral. Make
   the benchmark the merge gate, so a declaratively-authored peripheral cannot
   land unless it matches the silicon ground truth. This is where authoring cost
   and fidelity are kept honest together.

4. **Coverage push.** Convert the regular peripherals (UART/GPIO/SPI/I2C/timer
   families) to behaviour specs, retiring duplicated Rust. Reserve Rust for the
   complex peripherals. Report the KPI per family.

## Guardrails

- **Fidelity is non-negotiable.** Every declaratively-authored peripheral must
  pass the benchmark before it ships; the gate (phase 3) enforces this. Cheaper
  authoring that lowers fidelity defeats the entire purpose.
- **Escape hatch always open.** Anything the schema cannot express stays in Rust.
  The interpreter is the default path, not the only path.
- **Agent-assisted, not agent-trusted.** Filling a behaviour spec is a good fit
  for an agent, but the benchmark gate — not the agent's confidence — decides
  whether it is correct.
