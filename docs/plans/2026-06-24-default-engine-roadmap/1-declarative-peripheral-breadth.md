# Plan 1 — Breadth via the declarative peripheral layer

INTERNAL planning doc. Lives OUTSIDE the repo (standing rule). This is design
only; no code here is applied. All line citations are against the clean
`origin/main` checkout at `a876d471` (verified, not assumed).

Goal: turn a handful of supported chips into broad multi-vendor coverage WITHOUT
the per-peripheral cost of hand-written Rust, and WITHOUT lowering fidelity. The
declarative layer already exists and carries real load (esp32c3/esp32s3 ship
several peripherals through it). The work is to widen its *behaviour* vocabulary
just enough to absorb the regular peripheral families, teach the ingestor to
pre-fill what it can infer, and put a silicon-fidelity gate on the authoring loop
so cheaper authoring can never mean a looser model.

---

## CORRECTIONS vs the prior (stale-branch) draft

The earlier draft was written against `feat/cli-ingest-svd` (314 commits behind
main) and got four load-bearing facts wrong. On THIS main checkout:

1. **There IS a per-peripheral `clock:` field.** `PeripheralConfig.clock:
   Option<ClockGate>` (`crates/config/src/lib.rs:108-110`), `ClockGate { reg,
   bit }` (`lib.rs:91-96`), used across `configs/chips/stm32f103.yaml` (GPIOA,
   USART1, AFIO, bxCAN) and `stm32l476.yaml` (GPIOA/B/C, USART1/2, SPI1). The
   stale draft asserted "There is **no** per-peripheral `clock:` field." Wrong.

2. **Clock gating is BUS-level and peripheral-agnostic.** `is_peripheral_clocked`
   (`crates/core/src/bus/mod.rs:1216-1235`) is checked in every accessor
   (`crates/core/src/bus/accessors.rs:43,54,170,231,304,340,392`) BEFORE the
   access reaches the device: an unclocked gated peripheral reads 0 / drops
   writes regardless of whether it is Rust or `GenericPeripheral`. The gate is
   resolved once at build time (`resolve_clock_gates`, `mod.rs:1637-1671`,
   called from `from_config.rs:622`). Consequence that reshapes the whole plan:
   the "access an unclocked peripheral and it doesn't come alive" fidelity story
   is **already data-driven and already works for declarative peripherals** —
   it is NOT a vocabulary gap and NOT a reason to keep a peripheral in Rust. The
   stale draft called the RCC enable→status rule "the canonical instance of the
   vocabulary gap, today solved only in Rust." For the *consumer* side that is
   now false. (The *producer* side — RCC computing its own RDY bits from its own
   enable bits — is still Rust; see §1.1.)

3. **The sim-side fidelity oracle already exists and routes through
   `from_config`.** `crates/hw-oracle/tests/stm32f1_mmio_diff.rs` has FOUR
   non-ignored, non-hardware tests — `f1_reset_sim_only` (:1121),
   `f1_mmio_sim_only` (:1144), `f1_parity_sim_only` (:1167), `f1_sweep_sim_only`
   (:1207) — that build a full `SystemBus` from `stm32f103.yaml` via
   `build_sim_bus` (:1074) and replay the silicon-pinned `ResetCase`/`MmioCase`
   tables (`sim_reset_read` :1090, `sim_masked_read` :1101). The live-board
   compare is the only `#[cfg(feature = "hw-oracle-stm32")]` part (:1225). The
   stale draft claimed declarative peripherals have "**zero** silicon-fidelity
   coverage" and proposed building a brand-new `descriptor_oracle` harness from
   scratch. The harness exists; the gap is that no declarative peripheral is
   wired into a chip YAML's *tested* path yet, and these sim tests live in
   `tests/` so they run in `core-full` (post-merge), not the `--lib`-only
   `core-integrity` pre-merge gate. So Phase 0 shrinks to "make the existing sim
   oracle accept a declarative drop-in + move it pre-merge," not "build a new
   harness."

4. **`FieldDescriptor` has no per-field semantics.** It is `{ name, bit_range,
   description }` only (`crates/config/src/lib.rs:277-282`) — no `access`, no
   per-field reset, no hooks. The stale draft proposed hanging a `derived` block
   on `FieldDescriptor`; there is nothing there to hang it on cleanly. The new
   schema (§4) puts computed-read state at the **register** level instead.

Carried-over facts that re-verified TRUE: the BusFault claim is false on the
generic path (§Risks); `on_read`/`on_write` are parsed then dropped (§1.3);
svd-ingestor emits `timing: None` and never fills `on_read`/`on_write`
(`crates/svd-ingestor/src/lib.rs:123,366-367`); `examples/f103-fidelity-bench/`
exists and is a real LabWired-only false-pass bench (4/4 cases, README pins the
result).

---

## 0. Ground truth (verified against this checkout)

What the declarative layer does today
(`crates/core/src/peripherals/declarative.rs`):

- `GenericPeripheral` is built from a `PeripheralDescriptor`, auto-sizes its
  `Vec<u8>` to the highest register end, initialises every register to its
  `reset_value` (`declarative.rs:39-91`).
- Byte and 32-bit reads/writes with byte-granular reconstruction
  (`read`/`write`/`read_u32`/`write_u32`, `declarative.rs:186-349`).
- Access permissions: write to `ReadOnly` is dropped (`:220`, `:302`); read of
  `WriteOnly` returns 0 (`:192-193`, `:264`). It does **not** raise a fault on a
  violation — it silently no-ops (`grep -n BusFault crates/core/src/bus/accessors.rs
  crates/core/src/peripherals/declarative.rs` → nothing). `SimulationError`
  *has* `MemoryViolation(u64)` (`crates/core/src/lib.rs:48-49`) — the fault type
  exists; the generic path just never returns it.
- Side-effects: `read_action: clear` zeroes on read (`:200-204`, `:276-283`);
  `write_action` WriteOneToClear / WriteZeroToClear (`:227-241`, `:312-332`).
  Enum source `crates/config/src/lib.rs:286-299`.
- Timing: periodic + delayed `SetBits`/`ClearBits`/`WriteValue`, optionally
  raising a named IRQ via the `interrupts` map (`:97-182`, `:365-405`; config
  `TimingTrigger`/`TimingAction` at `crates/config/src/lib.rs:313-337`).
- `on_read`/`on_write` (`SideEffectsDescriptor`, `config/src/lib.rs:308-310`)
  are parsed and **dropped** — `declarative.rs` never reads them. The IR→config
  conversion hard-codes them to `None` (`config/src/lib.rs:432-433`). The
  ingestor sets them `None` (`svd-ingestor/src/lib.rs:366-367`).
  `docs/declarative_registers.md:36,50` promises a `HookHandler` trait that does
  not exist. Dead schema slots.

What feeds descriptors:

- `svd-ingestor` generates a `PeripheralDescriptor` from a CMSIS-SVD device:
  register map, sizes, access, reset values, fields, and the two side-effect
  enums it can read from SVD (`readAction=clear` → `ReadAction::Clear`;
  `modifiedWriteValues=oneToClear/zeroToClear` → the write actions) —
  `crates/svd-ingestor/src/lib.rs:310-407`. It always emits `timing: None`
  (`:123`) and never fills `on_read`/`on_write`.
- Dispatch is in `crates/core/src/bus/from_config.rs` (the task's "~3 sites"):
  per-family factories first (`esp32s3::factory`, `nrf52::factory`,
  `from_config.rs:125-135`), then `generic_factory::try_build` (`:137-142`),
  then the residual `match canonical_type` (`:146`) whose tail arms are the
  three descriptor loaders: `"declarative"` (loads a descriptor YAML by
  `config.path`, `:317-343`), `"strict_ir"` (loads Strict-IR JSON, `:344-408`),
  `"strict_ir_internal"` (inline IR converted during ingest, `:409-420`). All
  three construct a `GenericPeripheral`. Everything else in that match plus the
  two factories is hand-written Rust.

What is hand-written and stays so for now (with files): uart
(`peripherals/uart.rs`, dispatched `from_config.rs:147-170`), gpio
(`:171-212`), i2c (`:213-260`), rcc (`generic_factory.rs:38`), timer/spi/flash/
pwr/exti/dma/bxcan/fdcan/usb_otg/rng/crc/rtc/iwdg/etc.
(`generic_factory.rs:27-269`).

Clock gating — corrected (see Corrections #2). Already data-driven at the bus
layer; works for declarative peripherals out of the box once a `clock:` is
declared. The hand-written RCC still *computes its own* CR ready bits
(`classic_cr_ready`, `crates/core/src/peripherals/rcc.rs:69-82`: HSION→HSIRDY,
HSEON→HSERDY, PLLON→PLLRDY). That producer-side computed-status rule is the real
residual instance of the vocabulary gap (§1.1).

Fidelity coverage + CI gate, as they stand:
- The mmio_diff sim tests exist for F1 (`stm32f1_mmio_diff.rs`), F4
  (`stm32f4_mmio_diff.rs`), L4 (`l476_mmio_diff.rs`), L0 (`stm32l0_mmio_diff.rs`),
  H5 (`h563_mmio_diff.rs`), nRF52 (`nrf52_mmio_diff.rs`), and the
  esp32c3/esp32s3 reset-conformance tests. They build a `SystemBus` from the
  shipped chip YAML and replay silicon-pinned `ResetCase`/`MmioCase` constants.
  No test currently asserts a *declarative* re-author against one of these
  tables (the declarative peripherals in esp32 ship without a pinned case table
  in these mmio_diff files).
- Ground truth = silicon-confirmed constants hard-coded in the test source (read
  on a real board over SWD), e.g. `RCC_CR_RESET = 0x0000_4A83`
  (`stm32f1_mmio_diff.rs:227`), `ResetCase` (`:217-223`), `MmioCase` (`:410-417`).
- Merge gate `core-integrity` runs `cargo test --workspace --lib` only
  (`.github/workflows/core-ci.yml:63`); integration/hw-oracle (incl. the
  sim-only mmio tests) run post-merge in `core-full` `cargo test --workspace`
  (`:104`) plus the tier-1 matrix + ratchet (`:118`). The live-silicon compare
  is hardware-gated (`--features hw-oracle-stm32 --ignored`) and in neither lane.
- A behavioural coverage ratchet already exists: `crates/cli/src/coverage.rs`
  (Modelled/Indeterminate/Unmodelled classification, `:303-305`) +
  `crates/cli/tests/svd_coverage_ratchet.rs`, snapshot `docs/coverage/
  esp32s3-coverage.json`. The probe uses `set_clock_gating_bypass`
  (`mod.rs:1252`) so it can read a register's modelling independent of its clock.

---

## 1. The behaviour-vocabulary gap (precise catalogue)

The regular, family-wide patterns that currently force a Rust model. Each is
pinned to a hand-written workhorse so the target is silicon-validated code, not
invented. (Clock-gated *accessibility* is deliberately NOT in this list — it is
already data-driven; see Corrections #2.)

### 1.1 Computed / gated status flag (read value derived from other state)
- UART status read assembles `RXNE` (bit 5) live from RX-buffer occupancy and
  returns a `status_ready_value()` (`crates/core/src/peripherals/uart.rs:422-432`,
  `549-551`). The bench pins that an unenabled UART pends nothing
  (`uart.rs:778-787`).
- RCC CR ready bits computed from enables: `RDY = f(ON)` (`classic_cr_ready`,
  `rcc.rs:69-82`). This is the *producer-side* computed status — RCC deriving
  its OWN register, not the bus gating a consumer.
- I2C `TXE=1 at reset`, cleared by writing the data register (`i2c.rs:374,431`).

Today the generic path can only return the stored byte (`declarative.rs:196-208`).
No notion of "this field's read value is derived."

### 1.2 FIFO / data-register push-pop with level flags
- UART DR read pops one byte from the RX queue (`uart.rs:433-439`); DR write
  pushes to TX (`uart.rs:463-464`, `push_tx` :377). RXNE is the not-empty flag
  (`uart.rs:425-429`).
- I2C "RXNE clears on DR read" latched for the next tick (`i2c.rs:231,264`).

The generic path treats DR as a plain memory cell; reading returns the last
written byte, not the head of a queue.

### 1.3 Cross-register conditional readback / write-clears-elsewhere
- Writing the I2C data register clears `TXE`+`TXIS` in the status register
  (`i2c.rs:431`); START sets `BUSY` in SR2, STOP clears it (`i2c.rs:408-411`).
- UART: writing DR with `CR3.DMAT` set arms a DMA-TX flag changing later status
  (`uart.rs:466-468`).

`write_action` only edits the *same* register being written
(`declarative.rs:227-241`). The timing engine *can* touch another register but
only on a fixed delay/periodic trigger with constant bit-sets (`TimingAction`,
`config/src/lib.rs:333-337`) — it cannot express "on write of value X to A,
immediately recompute B." Note: a `Write` trigger with `delay_cycles: 0` still
fires through `tick()`, not synchronously in `write()` (the event is queued at
`declarative.rs:126-132` and only applied in `tick` at `:365-405`) — so even the
"immediate" path is a tick late relative to a Rust model that mutates B inline.

### Where to draw the line (escape hatch)
Full bus state machines (I2C master sequencing across SR1/SR2,
`i2c.rs:279-320`), DMA descriptor walks, CAN/FDCAN mailbox+RAM layout, USB
endpoint engines, crypto/RNG cores are **not** regular and stay in Rust. The
vocabulary targets §1.1–§1.3 only — the register-level patterns common to
UART/GPIO/SPI/I2C-register-layer/basic-timer, not their full transactional
behaviour.

---

## 2. KPI — time-to-faithful-peripheral (TTFP)

Definition: wall-clock + human-edit time from "vendor SVD in hand" to "a
declaratively-authored model of that peripheral that passes its silicon-fidelity
oracle." Measured per peripheral. A phase counts as progress only if it lowers
TTFP **and** the fidelity gate (§3 Phase 3) stays green.

Two existing artefacts make TTFP measurable cheaply:
- The coverage matrix + ratchet (`crates/cli/src/coverage.rs:303-305`,
  `crates/cli/tests/svd_coverage_ratchet.rs`, snapshot
  `docs/coverage/esp32s3-coverage.json`). It classifies each SVD register as
  Modelled / Indeterminate / Unmodelled. Extend "Modelled" from "register
  present / storage cell" to "register behaviourally covered" (Phase 2) so TTFP
  has a numeric proxy.
- Secondary proxy: **lines of hand-written Rust retired per family** (Phase 4).

Report both as a small table at the end of every phase PR.

---

## 3. Phase plan

Each phase independently shippable, gated, reversible. Do NOT migrate a family
(Phase 4) before its oracle drop-in exists (Phase 0/3).

### Phase 0 — Make the EXISTING sim oracle accept a declarative drop-in

Problem (corrected): the sim-side oracle exists (`build_sim_bus` +
`sim_reset_read`/`sim_masked_read`, `stm32f1_mmio_diff.rs:1074-1118`) and runs
through `from_config`, but (a) no declarative peripheral is wired into a tested
chip-YAML path against one of these tables, and (b) the sim-only tests live in
`tests/` so they only run post-merge in `core-full`.

Deliverable: a tiny seam that lets the same `ResetCase`/`MmioCase` table run
against a chip config in which one peripheral's `type` is flipped from its Rust
type to `declarative` (pointing at a descriptor YAML). Two concrete moves:

1. Parameterise `build_sim_bus` (or add `build_sim_bus_with_overlay`) to load an
   alternate system manifest whose peripheral block swaps the target peripheral
   to `type: declarative`. Everything downstream (`from_config`, clock-gate
   resolution, accessors) is already type-agnostic — no engine change.
2. Add one bring-up test `f1_declarative_dropin_sim_only` that asserts a
   reset-value-only declarative block passes a subset of `RESET_CASES`, and that
   a deliberately-wrong `reset_value` in the descriptor FAILS the same table.
   This proves the seam has teeth before it gates anything.

No new harness, no new ground-truth format — reuse `ResetCase`/`MmioCase` and
`sim_*_read`. The board-attached run (`#[cfg(feature = "hw-oracle-stm32")]`)
stays the periodic deeper check, unchanged.

Files touched: `crates/hw-oracle/tests/stm32f1_mmio_diff.rs` (parameterise +
new test), a fixture descriptor YAML under `configs/peripherals/` or
`crates/hw-oracle/tests/fixtures/`.

Exit: trivially-correct declarative block passes; wrong reset value fails.

### Phase 1 — Extend the behaviour vocabulary (smallest set for §1.1–§1.3)

Add three declarative primitives, kept as data not embedded code (the whole
point is YAML, not Rust). All schema additions `#[serde(default)]` so every
existing descriptor — incl. the IR→descriptor path (`config/src/lib.rs:411-450`)
and the esp32 descriptors — parses unchanged; absent means "plain storage cell."

1. **Computed register read (§1.1).** A new optional `derived` block at the
   **register** level (NOT on `FieldDescriptor` — it has no semantics slot;
   Corrections #4). `derived` carries a list of `{ field_bits, equals_one_when:
   <predicate> }` so individual bits of a register can be computed while others
   stay stored. Predicate = a bounded boolean AND/OR of
   `field_set("REG.bitrange")` / `field_clear(...)` / `queue_nonempty("DR")` /
   `queue_full("DR")`. A closed grammar evaluable in a `match` — never `eval`.
   Expresses `RXNE = queue_nonempty(DR)` and the RCC `RDY = field_set(ON)` rule.

2. **Queue register (§1.2).** New optional `queue` block on a register:
   `queue: { direction: rx|tx, depth: N, nonempty_flag: "SR.bit",
   full_flag: "SR.bit" }`. Read of an `rx` queue register pops + updates flags;
   write of a `tx` queue register pushes + updates flags. Depth-1 = classic
   non-FIFO DR; depth-N = real FIFO. Backing store for that register moves from
   the raw `Vec<u8>` cell into a descriptor-owned queue inside
   `GenericPeripheral`.

3. **Reactive write hook (§1.3).** Make a `Write` trigger with `delay_cycles: 0`
   apply **synchronously inside `write()`/`write_u32()`** rather than via the
   inflight queue, and allow an action *list* per trigger (today one `action`,
   `config/src/lib.rs:344`). This expresses "writing DR clears SR.TXE" with no
   one-tick lag. Reuse the existing cross-register `SetBits`/`ClearBits`
   machinery (`declarative.rs:138-182`); the change is (a) inline application for
   `delay_cycles == 0`, (b) `action: TimingAction` → `actions: Vec<TimingAction>`.

Decide `on_read`/`on_write` (`config/src/lib.rs:308-310`): **delete** them and
the `HookHandler` prose in `docs/declarative_registers.md:36,50`. A free-string
hook re-opens the Rust escape hatch in disguise and defeats "data not code." Keep
the hatch explicit and coarse (whole peripheral stays Rust). This also removes
the dead slots the IR conversion and ingestor already null out.

Interpreter changes (`declarative.rs`): evaluate `derived` in `read`/`read_u32`;
route queue registers through the queue store; apply `delay_cycles == 0` write
actions inline.

**Fix the BusFault claim in this phase.** Make access-permission violations on
the generic path return `SimulationError::MemoryViolation` (or whatever the
hand-written workhorses do — audit; today none of them fault either, so decide
the single correct silicon behaviour and apply it uniformly) instead of silently
no-op'ing (`declarative.rs:192,220,264,302`). Gate with an oracle case. Update
`docs/declarative_registers.md` in the same PR.

Proof of phase: re-serve the **F1 UART** from a descriptor (RXNE-from-queue, DR
pop/push, derived status, the unenabled-pends-nothing rule — the last comes free
from the existing `clock:` gate, Corrections #2) and pass it through the Phase 0
seam against the UART silicon-pinned cases, with `uart.rs` out of the dispatch
path for that test. Record TTFP(F1 UART).

Risk if (1)–(3) prove too narrow: that is *signal*, not failure — the family
belongs behind the escape hatch. Record which pattern overflowed and STOP
expanding the DSL; do not grow it toward Turing-completeness.

### Phase 2 — Teach svd-ingestor to pre-fill inferable behaviour

`svd-ingestor` already lifts reset/access/fields and the two SVD-native
side-effect enums (`crates/svd-ingestor/src/lib.rs:310-407`). Extend it to emit
the new primitives where SVD or naming conventions make them inferable, leaving
genuine semantic gaps for a human/agent:

- **Enable→status inference (§1.1).** When a CR field is named
  `EN`/`UE`/`SPE`/`PE` (family-standard enable names) and a status register has
  an obvious ready/idle flag, emit a `derived: equals_one_when: field_set(<EN>)`
  skeleton, tagged inferred/low-confidence.
- **Queue inference (§1.2).** A register whose SVD name/`dataType` marks it a
  data register (`DR`/`TDR`/`RDR`/`FIFO`) → emit a `queue` block; depth from SVD
  if a FIFO depth is declared, else 1.
- **Clock-gate inference.** When the SVD/naming ties a peripheral to an RCC
  enable bit, emit the `PeripheralConfig.clock:` mapping (`config/src/lib.rs:91`)
  so accessibility gating comes for free (Corrections #2). This is new vs the
  stale draft, which didn't know `clock:` existed.
- **Side-effect coverage.** Audit `modifiedWriteValues`/`readAction` SVD variants
  beyond the two mapped at `svd-ingestor/src/lib.rs:371-388`.

Output is a *draft* descriptor: every inferred behaviour carries a
`provenance: inferred|verified` field (new, `#[serde(default = verified)]` so
hand-written stays verified) so the gate treats inferred as unverified until an
oracle confirms. The ingestor never asserts correctness — it lowers starting
cost.

Coverage upgrade: extend `coverage.rs` so a register counts Modelled only when
its *behaviour* (not mere presence) is covered — a verified primitive or a plain
storage cell. This makes the ratchet (`svd_coverage_ratchet.rs`) guard
behaviour and gives TTFP a number.

Files: `crates/svd-ingestor/src/lib.rs`, `crates/cli/src/coverage.rs`, the
coverage snapshot JSON, `crates/config/src/lib.rs` (`provenance` field).

Measure TTFP before/after on 2–3 peripherals across two vendors; report the delta.

### Phase 3 — Fidelity gate on the authoring loop

Non-negotiable: a declaratively-authored peripheral cannot land unless it matches
silicon ground truth. The gate, not the author's (or an agent's) confidence,
decides correctness.

- **Hard gate (blocking, pre-merge).** Any descriptor that ships in a chip/board
  config and has a silicon-pinned case table MUST pass its sim-side oracle
  (Phase 0 seam, no hardware). Any `provenance: inferred` primitive that lacks a
  passing oracle is a **hard error** — inferred behaviour may not reach a
  shipped config unverified.
- **Ratchet (non-regression).** Behavioural-coverage numbers may not drop
  (reuse `svd_coverage_ratchet.rs`).

Placement — this is the real CI change. The sim-only mmio tests today run in
`core-full` post-merge (`.github/workflows/core-ci.yml:104`), not the
`--lib`-only `core-integrity` pre-merge gate (`:63`). The descriptor oracle needs
no hardware, so it CAN and MUST move pre-merge — fidelity blocking merge is the
standing false-pass-prevention wedge. Add a small dedicated fast PR job that runs
just the sim-only oracle tests (e.g. `cargo test -p labwired-hw-oracle --test
stm32f1_mmio_diff -- f1_*_sim_only` plus the new declarative drop-in test), no
`hw-oracle-stm32` feature. Keep it a named, visible check so the ~2–3 min `--lib`
gate stays fast. Files: `.github/workflows/core-ci.yml`.

Agent-assisted authoring is welcome above this line: an agent ingests, fills the
DSL, proposes a descriptor; the oracle is what lets it land — the
cert-evidence / provable-false-pass-prevention posture applied to authoring.

### Phase 4 — Migrate the regular families, retire duplicated Rust

Per family, lowest transactional complexity first: GPIO → basic/general timer →
SPI → UART → I2C *register layer only*. For each:

1. Author the family descriptor(s) using Phase-1 primitives + Phase-2 ingest,
   including the `clock:` gate so accessibility fidelity comes free.
2. Pass the per-part oracle (Phase 3) against the *same* silicon-pinned
   `ResetCase`/`MmioCase` tables the hand-written model passes — byte-identical.
3. Switch the dispatch. The GPIO/UART/I2C arms are in the residual match
   (`from_config.rs:147-260`); timer/spi/flash/etc. are in
   `generic_factory.rs:27-269`. For a migrated type, construct a
   `GenericPeripheral` from the family descriptor, keeping the per-part config
   knobs (`cr3_mask` `from_config.rs:161-166`; gpio `num_pins`/`reset_moder`
   `:184-208`; profiles via `parse_profile_or_default`) expressed as descriptor
   parameters or as selected descriptors.
4. Delete the retired Rust module once its oracle passes on the generic path and
   the arm is switched. Report Rust-LoC retired.

A family with no silicon-pinned case table is not eligible — write the table
first, or lift the `expect` constants from the values the passing Rust model is
already validated against (those came from real silicon over SWD). Never
synthesise `expect` for unvalidated cases — that manufactures a false pass.

Reserve Rust permanently for: DMA (`dma.rs`), bxCAN/FDCAN, USB-OTG, crypto/RNG
state, RCC ready-bit computation (the §1.1 producer rule, if the DSL doesn't
cleanly cover `classic_cr_ready`), and any peripheral whose §1 pattern overflowed
the DSL in Phase 1. The interpreter is the **default** path; Rust is the
explicit, coarse escape hatch.

Do NOT migrate a family before steps 1–2 are green.

---

## 4. Schema changes (summary, all additive + `#[serde(default)]`)

| Where | Add / change | Purpose |
|---|---|---|
| `RegisterDescriptor` | `derived: Vec<{ field_bits, equals_one_when: <bounded predicate> }>` | computed/gated status bits (§1.1) — register-level, since `FieldDescriptor` has no semantics slot |
| `RegisterDescriptor` | `queue { direction, depth, nonempty_flag, full_flag }` | FIFO/DR push-pop + level flags (§1.2) |
| `TimingDescriptor` | `action: TimingAction` → `actions: Vec<TimingAction>`; `delay_cycles == 0` applied inline in `write()` | cross-register reactive write, no tick lag (§1.3) |
| `RegisterDescriptor` | `provenance: inferred \| verified` (default verified) | gate treats inferred as unverified |
| `SideEffectsDescriptor` | **remove** `on_read` / `on_write` | dead free-string hooks; close the per-register Rust backdoor; also drop the `HookHandler` prose in docs |

All backward compatible: existing descriptors and the IR→descriptor conversion
(`config/src/lib.rs:411-450`) parse unchanged.

---

## 5. Guardrails

- **Fidelity is the gate, not a comment.** Phase 3's blocking sim oracle enforces
  it; cheaper authoring that lowers fidelity cannot merge. Same
  false-pass-prevention wedge applied to the authoring path.
- **The DSL stays bounded.** Predicates evaluate in a `match`, never `eval`. A
  pattern that overflows the three primitives goes behind the escape hatch — do
  NOT grow the DSL toward a scripting language.
- **Escape hatch stays open and coarse.** Whole-peripheral Rust remains a
  first-class dispatch path (`from_config.rs` match + the two factories).
  Removing per-register `on_read`/`on_write` keeps the hatch *explicit* (opt the
  whole block into Rust) instead of smuggling code into a descriptor string.
- **Build ON the existing layer.** Reuse `GenericPeripheral`,
  `PeripheralDescriptor`, the `clock:` gate, the coverage ratchet, and the
  existing `build_sim_bus`/`ResetCase`/`MmioCase` harness. Do NOT fork a second
  descriptor type or a second fidelity harness (the stale draft's mistake).
- **Correct the docs as we go.** `docs/declarative_registers.md:36,50`
  overstates BusFault enforcement and a `HookHandler` that does not exist; each
  phase touching a claim fixes the doc in the same PR.

---

## 6. Risks

- **BusFault gap (live, not hypothetical).** The generic path silently no-ops
  permission violations (`declarative.rs:192,220,264,302`) while the doc claims a
  fault. Until Phase 1 fixes this, a declarative re-author is *less* faithful on
  illegal accesses than intended — a silent fidelity regression hiding inside a
  breadth win. Fix it first, gated. (Audit whether the hand-written workhorses
  fault at all; if not, decide the one correct silicon behaviour and apply it
  uniformly so the doc and code finally agree.)
- **Byte-by-byte write semantics.** `GenericPeripheral::write` triggers per byte
  with a shifted value (`declarative.rs:243-251`); the synchronous multi-register
  reactive action (§1.3) must be defined on the `write_u32` path or accumulate,
  or a HAL doing byte stores mis-fires. Pin with an oracle case in Phase 1.
- **Inference over-reach.** Ingestor-guessed `derived`/`queue`/`clock` blocks can
  be plausibly wrong (an `EN`-named field that doesn't gate the flag; a wrong RCC
  bit). `provenance: inferred` + the Phase-3 hard gate contain it: inferred ≠
  trusted until an oracle confirms.
- **Case-table availability.** Some families lack a silicon-pinned table.
  Mitigation: lift `expect` constants from the values the currently-passing Rust
  model is validated against (real silicon over SWD), then hold the generic path
  to the same table. Sound ONLY for cases the Rust model itself passes on real
  silicon — never synthesise `expect` for unvalidated cases.
- **Producer-side computed status may resist the DSL.** `classic_cr_ready`
  (`rcc.rs:69-82`) couples three (ON,RDY) pairs with source-readiness gating; if
  the §1.1 predicate grammar can't express it cleanly, RCC stays Rust (it is a
  one-off, not a breadth family — acceptable).

---

## 7. Sequencing & exit per phase

| Phase | Exit criterion | KPI recorded |
|---|---|---|
| 0 | declarative drop-in passes a correct reset block via `build_sim_bus`; wrong reset value fails | — (seam only) |
| 1 | F1 UART re-served from a descriptor passes the UART silicon-pinned cases; BusFault enforced; `on_read/on_write` removed | TTFP(F1 UART) |
| 2 | ingestor emits inferred `derived`/`queue`/`clock` + provenance; behavioural coverage metric live | ΔTTFP on 2–3 peripherals, 2 vendors |
| 3 | sim oracle + drop-in test moved to the pre-merge lane; inferred-unverified is a hard error | gate green on all shipped descriptors |
| 4 | each migrated family passes its oracle on the generic path; Rust module deleted | Rust-LoC retired / family |
