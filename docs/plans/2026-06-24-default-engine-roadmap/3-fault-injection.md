# Plan Item #3 — Fault Injection as a First-Class Feature

Internal implementation plan. Lives OUTSIDE the repo. No code changes implied by
this document. Grounded read-only against a clean checkout of `origin/main`
(`/tmp/.../scratchpad/wt-main`, tip `a876d471`). Every grounding claim cites
`file:line` on THIS checkout.

## 0. Corrections vs the prior stale-branch draft

The previous draft of this file read branch `feat/cli-ingest-svd` (314 commits
behind main) and is wrong on three counts. Re-verified on main:

- **The `clock:` field exists in the chip YAMLs.** `configs/chips/stm32f103.yaml:32`
  (`clock: { reg: "apb2enr", bit: 2 }` for GPIOA), `:52` (USART1), `:83` (AFIO),
  `:142` (bxCAN); `configs/chips/stm32l476.yaml:39,48,57,93,104,148`. It is a
  typed config field — `ClockGate { reg: String, bit: u8 }` at
  `crates/config/src/lib.rs:90-96`, wired through `PeripheralConfig::clock`
  (`crates/config/src/lib.rs:110`). So `missing_clock` is **NOT** gated/deferred:
  it lowers directly onto removing/clearing that enable binding (see §4.3).
- **`examples/f103-fidelity-bench/` exists in-repo** (not only in the private
  `labwired-core-falsepass`). It ships `control-smoke.yaml`, `clockbug-smoke.yaml`,
  `gpiobug-smoke.yaml`, `rambug-smoke.yaml`, `system.yaml`, `system-nogate.yaml`,
  `run-benchmark.sh`, `README.md`, and `firmware/{main.c,startup.c,bench.ld,Makefile}`.
- **The bus accessors moved.** They are no longer in `bus/mod.rs` at the line
  numbers the old draft cites; they live in `crates/core/src/bus/accessors.rs`
  (see §2.1). All old `bus/mod.rs:24xx` cites are stale — ignore them.

One NEW correction this draft also makes vs the old draft's "violation faults
into HardFault" assumption: it does **not**. See §2.1 — a violation aborts the
run; handler-vectoring is a phased CPU task here.

## 1. Problem & goal

Safety-relevant firmware must be *shown to detect and handle* hardware faults.
Today LabWired only demonstrates this ad hoc. `examples/f103-fidelity-bench/`
ships **four separate firmware build variants** — the bug is baked into the
firmware, one line per case (`README.md:23-27`): `control` (correct),
`clockbug` (drops `RCC_APB2ENR.USART1EN`), `gpiobug` (drives GPIOA without
`IOPAEN`), `rambug` (stores 4 KB past the 20 KB SRAM). Each is asserted only
*indirectly* via a `uart_contains: "BENCH_*_OK"` marker (`clockbug-smoke.yaml:13`,
`gpiobug-smoke.yaml:13`, `rambug-smoke.yaml:13`). That is backwards for cert: the
**device under test must stay fixed and the environment is what we perturb.**

Goal: a declarative way to inject a fault into the simulated silicon, run the
*unmodified* firmware, and emit a per-fault verdict — did the firmware behave
safely? Fault kinds in scope: stuck-at register bits, wrong reset values,
missing/disabled clock enables, RAM-bound / access-permission violations, memory
corruption, delayed/never-arriving IRQs, peripheral error/timeout states.

Hard constraint: an injected fault must be **silicon-plausible** — a bit that
won't toggle, a clock never enabled, a peripheral that NAKs forever — not fantasy
faults real silicon can't produce. The schema and the engine hooks both enforce
this (§5 guardrails).

## 2. What the codebase already gives us (cite-grounded)

### 2.1 The bus access path is the single memory-fault chokepoint
Every CPU load/store funnels through the `Bus` impl for `SystemBus` in
`crates/core/src/bus/accessors.rs`: `read_u8`/`write_u8` (`:13`, `:79`),
`read_u16`/`write_u16` (`:211`, `:331`), `read_u32`/`write_u32` (`:256`, `:359`).
Peripheral MMIO is dispatched via `find_peripheral_index(addr)` then
`p.dev.read/write(addr - p.base)` (`:42-47`, `:169-180`). Unmapped access returns
`Err(SimulationError::MemoryViolation(addr))` (`:76`, `:192`), env-gated for
tracing by `LABWIRED_TRACE_VIOLATIONS` (`:73`, `:186`). RCC clock-gating is
already enforced here: `is_peripheral_clocked(idx)` (`bus/mod.rs:1216`) makes an
unclocked peripheral read 0 / drop writes (`accessors.rs:43-44`, `:170-174`).

**Violation behaviour — verify and phase (the prior reviewer was right).** A
`MemoryViolation` does **NOT** vector into the firmware's HardFault handler.
Instruction *fetch* uses `?` (`cpu/cortex_m.rs:826`, `:830`), so a fetch
violation propagates up through `Machine::step` (`lib.rs:895`) → `run`/
`step_batch` (`lib.rs:1373`) and **aborts the whole simulation**; the CLI then
maps the `Err` to `StopReason::MemoryViolation` (`cli/src/main.rs:1319-1320`,
`:1663-1664`, `:1689-1690`). Worse, *data* loads/stores mostly swallow the
violation silently — `if let Ok(val) = bus.read_u32(...)` / `let _ =
bus.write_u32(...)` (`cpu/cortex_m.rs:1162`, `:1177`, `:1199`, `:1234`, etc.), so
an out-of-bounds store often produces **no** observable effect at all. There is
no HardFault escalation anywhere (`grep HardFault cpu/cortex_m.rs` → only
priority-table comments at `:66,:184,:213,:4178`). The `rambug` case's README
claim "the store faults (HardFault)" is therefore inaccurate as to mechanism: in
practice the bench's rambug FAILs because the marker never prints, not because a
handler ran.

Consequence for this plan: a `bound`/`permission` fault verdict can only key off
"sim aborted with `MemoryViolation`" today. **Making the violation vector into the
firmware's HardFault handler is a phased CPU task** (Phase 3, §6) — until then the
verdict vocabulary is limited to `sim_aborted` / `marker_absent`, not
`handler_ran`.

### 2.2 The declarative peripheral already does the timing/side-effect work
`GenericPeripheral` (`crates/core/src/peripherals/declarative.rs:28`) is built
from a `PeripheralDescriptor` and already implements everything the
peripheral-class faults need:

- **Reset values** seeded per register at construction
  (`declarative.rs:50-68`) from `RegisterDescriptor::reset_value`
  (`config/src/lib.rs:355`). → `wrong_reset_value` is a one-field mutation.
- **Access permissions** enforced: `ReadOnly` drops writes (`declarative.rs:220`),
  `WriteOnly` reads 0 (`:192`). → `permission_flip` flips a register's `access`.
- **Side-effects**: `SideEffectsDescriptor` with `read_action` (Clear) and
  `write_action` (W1C / W0C) (`config/src/lib.rs:302-311`, applied
  `declarative.rs:200-204`, `:227-241`, `:276-283`, `:312-332`).
- **Timing engine**: `TimingDescriptor { trigger, delay_cycles, action,
  interrupt }` (`config/src/lib.rs:340-347`). Triggers are `Write{register,
  value, mask}` / `Read{register}` / `Periodic{period_cycles}`
  (`:315-329`); actions are `SetBits` / `ClearBits` / `WriteValue`
  (`:333-337`); `check_triggers` arms an `InflightEvent` (`declarative.rs:97-136`),
  `tick()` counts `delay_remaining` down and on fire applies the action AND, if
  `interrupt` names an entry in `descriptor.interrupts`, pushes its IRQ number
  into `explicit_irqs` (`declarative.rs:365-405`, esp. `:378-384`). Periodic
  re-arms (`:387-394`).

This is the substrate. **Every timing/peripheral-class fault is a synthesised
`TimingDescriptor` or `SideEffectsDescriptor` mutation — we add no parallel
machinery.** A `delayed_irq` is a `TimingDescriptor` whose `delay_cycles` is
inflated; a `never_irq` strips the `interrupt` from the firing event (or never
arms it); a `peripheral_timeout`/`error_state` is a `Write`-triggered
`SetBits`/`ClearBits` that drives a status/error register into the stuck state.

### 2.3 The bus build path is where synthesised descriptors are injected
`SystemBus::from_config` (`bus/from_config.rs:18`) builds each peripheral; the
declarative path constructs `GenericPeripheral::new(desc)` from a
`PeripheralDescriptor` loaded from file (`from_config.rs:330,340`) or lowered from
IR (`:403,:405,:416,:418`). Clock-gates are resolved AFTER peripherals exist:
`bus.resolve_clock_gates(&merged_peripherals)` (`from_config.rs:622`), which maps
the symbolic `reg` name to a concrete RCC offset via `Rcc::enable_reg_offset`
(`bus/mod.rs:1637-1648`, `peripherals/rcc.rs:742`) and stores a
`ResolvedClockGate { reg_offset, bit }` (`bus/mod.rs:33-38`) on each
`PeripheralEntry::clock_gate` (`bus/mod.rs:56`). **This is the single point where
fault mutations are applied to descriptors / gates before the peripheral goes
live.**

### 2.4 The test runner + assertion evaluator is where verdicts attach
`labwired test` loads a `TestScript` (`config/src/lib.rs:653`: `schema_version` +
`inputs` + `limits` + `assertions`; loaded via `load_test_script`,
`:748`). The run loop is `execute_test_loop` (`cli/src/main.rs:1500`); after the
run it evaluates each `TestAssertion` (`config/src/lib.rs:643-649`) in the match
at `cli/src/main.rs:1739-1800` — variants `UartContains`, `UartRegex`,
`ExpectedStopReason`, `MemoryValue`, `UdsTester` — sets `all_passed` and writes a
`status` of `pass`/`fail`/`error` (`:1826-1832`) plus per-assertion
`AssertionResult { assertion, passed }` (`:1815-1818`, struct at `:628-629`).
`StopReason` already includes `MemoryViolation` (`config/src/lib.rs:578`), so
`expected_stop_reason: memory_violation` is assertable today.

### 2.5 A verdict-word contract already exists — reuse it, don't reinvent
The `Fuzz` subcommand already defines exactly the verdict pattern this plan needs:
a u32 verdict word at a fixed RAM address with `done_magic` / `fault_magic`
markers (`cli/src/main.rs:193-203`: `verdict_addr` default `0x2000_3000`,
`done_magic 0xC0DEF022`, `fault_magic 0xDEADFA17`). The firmware's own fault/panic
handler writes the magic; the harness reads it back. **The `verdict:` schema (§4.5)
reuses this contract and the existing `MemoryValue` assertion mechanism to read
it** — no new firmware ABI invented.

## 3. Design overview

Faults are declared in the **test YAML** (the perturbation lives with the test,
not the firmware), compiled at Phase 0 against silicon guardrails, lowered onto
existing hooks (§2.2/§2.3) during bus build, fired during the run, and judged by a
`verdict:` block that the assertion evaluator already knows how to read (§2.4/2.5).

The contract with the rest of the roadmap:
- **Builds ON #1 (declarative substrate).** Peripheral-class faults are
  `TimingDescriptor`/`SideEffectsDescriptor` mutations on `GenericPeripheral`
  (§2.2). No new peripheral kind.
- **Feeds #2 (cert evidence).** Each fault emits a `FaultEvidence` record
  (§4.6) — `fault_triggered: bool` + `verdict` + the resolved target — which is
  the exact contract #2's cert report ingests. **Fix this record's shape early
  (Phase 0)** so #2 and #3 don't diverge.

## 4. Schema

Two new top-level blocks in the v1.x `TestScript` (`config/src/lib.rs:653`):
`faults:` and `verdict:`. Bump `schema_version` to `"1.1"` and accept it in
`TestScript::validate` (currently hard-rejects anything but `"1.0"`,
`config/src/lib.rs:672-677`). `1.0` scripts are unaffected (`faults`/`verdict`
default-empty via `#[serde(default)]`).

### 4.1 `faults:` — a list of `FaultSpec`
```yaml
faults:
  - id: usart1_no_clock          # unique; named in verdict + evidence
    kind: missing_clock          # the fault taxonomy (§4.3)
    target: { peripheral: usart1 }   # resolved at Phase 0 (§5)
    trigger: at_start            # at_start | on_write{reg,value?,mask?} | on_read{reg} | after_cycles{n}
    # kind-specific params follow (see §4.3-4.4)
```

### 4.2 Target resolution
`target` is resolved (Phase 0) to a concrete `(peripheral_index, register?,
bit?)` against the built chip: `peripheral` matches `PeripheralConfig::id`;
optional `register` matches a `RegisterDescriptor::id` in that peripheral's
descriptor; optional `bit` / `bits` is validated against `RegisterDescriptor::size`
(`config/src/lib.rs:353`). Memory-class faults target an `address` (+ `size`)
instead of a peripheral. A target that doesn't resolve is a **hard config error**
(§5), never a silent skip.

### 4.3 Fault taxonomy → how each lowers onto an existing hook
| kind | params | lowers onto |
|---|---|---|
| `missing_clock` | `target.peripheral` | drop/clear that peripheral's resolved `clock_gate` so `is_peripheral_clocked` returns false (or its RCC enable bit is forced 0) — `bus/mod.rs:56,1216`, applied at `resolve_clock_gates` time (`from_config.rs:622`). The exact `clockbug`/`gpiobug` mechanism, now declarative. |
| `stuck_at_bit` | `target.{register,bit}`, `level: 0\|1` | a per-register write-mask applied in `GenericPeripheral::write*` so the bit can't change (new tiny hook: a `stuck_mask` on the descriptor, honoured in `declarative.rs:214-257,297-349`). |
| `wrong_reset_value` | `target.register`, `value` | overwrite `RegisterDescriptor::reset_value` before `GenericPeripheral::new` seeds it (`declarative.rs:50-68`). Zero engine change — a descriptor field edit. |
| `permission_flip` | `target.register`, `to: read_only\|write_only` | flip `RegisterDescriptor::access`; enforcement already exists (`declarative.rs:192,220`). |
| `bound_violation` | `target.address` | a synthetic always-trap window (or shrink a region) so a store there returns `MemoryViolation` from the accessor (`accessors.rs:192`). The `rambug` mechanism, declarative. |
| `permission_violation` | `target.address`, `deny: write\|read` | accessor returns `MemoryViolation` for the denied direction at that address. |
| `memory_corruption` | `target.address`, `value`/`xor`, `trigger` | a one-shot/periodic bus write at `address` injected from the per-tick service loop (model it like `service_hcsr04`'s bus-touch pattern, `bus/mod.rs:778-805`). |
| `delayed_irq` | `target.{peripheral,interrupt}`, `delay_cycles` | synthesise/inflate a `TimingDescriptor.delay_cycles` on that peripheral (`config/src/lib.rs:343`, fired `declarative.rs:372-384`). |
| `never_irq` | `target.{peripheral,interrupt}` | strip the `interrupt` from the firing event so `explicit_irqs` is never pushed (`declarative.rs:378-384`). |
| `peripheral_error_state` / `timeout` | `target.{peripheral,register}`, `bits`, `trigger` | a `Write`/`Periodic`-triggered `SetBits`/`ClearBits` `TimingDescriptor` that drives the status/error register into and holds it in the stuck state (`config/src/lib.rs:333-337`, applied `declarative.rs:138-182`). |

Only `stuck_at_bit`, `bound_violation`, `permission_violation`, and
`memory_corruption` need *new* engine code; all are thin additions at the two
chokepoints (§2.1, §2.3). The rest are pure descriptor/gate mutations on existing
machinery.

### 4.4 Triggers
Reuse the declarative `TimingTrigger` vocabulary (`config/src/lib.rs:315-329`)
verbatim for peripheral-class faults: `at_start` (apply during build),
`on_write{register,value?,mask?}`, `on_read{register}`, `after_cycles{n}`
(→ a `Periodic`/one-shot inflight event). No new trigger evaluator — `check_triggers`
already does this (`declarative.rs:97-136`).

### 4.5 `verdict:` — the safe-behaviour judgment
```yaml
verdict:
  # the firmware passes iff it detected/handled the fault safely:
  safe_when:
    - uart_contains: "FAULT_HANDLED"        # reuse TestAssertion variants verbatim
    - memory_value: { address: 0x20003000, expected_value: 0xDEADFA17 }  # the §2.5 verdict word
    - expected_stop_reason: memory_violation
  # optional: the run is INVALID (not a pass/fail) unless the fault actually fired:
  require_fault_fired: true                  # default true — the false-pass gate (§5)
```
`safe_when` entries are exactly `TestAssertion`s (`config/src/lib.rs:643-649`),
evaluated by the SAME match arm at `cli/src/main.rs:1739-1800`. The verdict block
is sugar that (a) groups the safe-behaviour assertions and (b) couples them to
`require_fault_fired`. This keeps one evaluator, not two.

### 4.6 `FaultEvidence` — the #2 contract (fix shape EARLY)
Per fault, emitted into the existing `TestResult` JSON (written by `write_outputs`,
`cli/src/main.rs:1874`):
```jsonc
{ "id": "usart1_no_clock", "kind": "missing_clock",
  "resolved_target": { "peripheral": "usart1", "rcc_reg_offset": "0x18", "bit": 14 },
  "fault_fired": true,            // observed to actually take effect (§5 gate)
  "verdict": "safe",              // safe | unsafe | invalid(fault_not_fired)
  "trigger_cycle": 4123 }
```
This record IS what #2's signable cert report consumes. Agree the field names in
Phase 0 with #2 so neither side re-cuts it later.

## 5. The "fault-must-actually-fire" gate (kill silent false-passes)

The single biggest risk in fault injection is a **silent no-op**: a fault that
never took effect, so the firmware "handled" a fault that wasn't there → a false
pass that is *worse* than no test. Two layers:

1. **Phase-0 config-side fault compiler (guardrails, before any run).** A new
   validation pass (sibling to `TestScript::validate`, `config/src/lib.rs:671`)
   run after the bus is built so it sees real peripherals:
   - target resolves to a real `(peripheral, register?, bit?)` / mapped address —
     else hard error (mirrors how `resolve_clock_gates` errors on an unmappable
     `reg`, `bus/mod.rs:1637-1648`);
   - bit/bits within `RegisterDescriptor::size` (`config/src/lib.rs:353`);
   - `wrong_reset_value` fits the register width (plausibility, not arbitrary);
   - `missing_clock` target actually HAS a `clock_gate` to remove (else the fault
     is structurally a no-op — reject, don't ship a dead test);
   - `delayed_irq`/`never_irq` target peripheral actually has that
     `interrupt` in its descriptor (`declarative.rs:379-381`).
2. **Runtime fired-observation (per fault).** Each lowered fault sets a
   `fired: bool` when its hook actually executes: the clock-gate drop is recorded
   the first time a gated access is suppressed; a `stuck_at_bit` records fired on
   the first masked write; a `bound`/`permission` violation records fired when the
   accessor returns the synthetic `MemoryViolation` (`accessors.rs:192`); a timing
   fault records fired when its `InflightEvent` fires (`declarative.rs:376`). If
   `require_fault_fired` (default true) and `fired == false`, the verdict is forced
   to `invalid` and the run FAILS regardless of `safe_when` — the test is broken,
   not the firmware safe.

This gate is the cert-grade differentiator: we *measure* that the perturbation
reached the silicon, we don't assume it.

## 6. Phases, files-touched

**Phase 0 — schema + compiler + evidence contract (config-only, no engine).**
- `crates/config/src/lib.rs`: add `FaultSpec`, `FaultKind`, `FaultTarget`,
  `Verdict`, `FaultEvidence`; add `faults`/`verdict` to `TestScript` (`:653`);
  bump+accept `schema_version "1.1"` in `validate` (`:672`); the fault-compiler
  validation entrypoint.
- Lock the `FaultEvidence` JSON shape WITH #2 here.
- Tests: target-resolution + guardrail-rejection unit tests (extend the
  `serde_yaml` round-trip tests already at `config/src/lib.rs:~930-951`).

**Phase 1 — lower the zero/low-engine fault kinds (the 80%).**
- `missing_clock`, `wrong_reset_value`, `permission_flip`, `delayed_irq`,
  `never_irq`, `peripheral_error_state/timeout` — all descriptor/gate mutations
  applied in `bus/from_config.rs` around `resolve_clock_gates` (`:622`) and at
  `GenericPeripheral::new` (`declarative.rs:39`). No new peripheral code.
- Wire fired-observation + emit `FaultEvidence` into `write_outputs`
  (`cli/src/main.rs:1874`); add the `verdict`/`require_fault_fired` evaluation
  beside the assertion loop (`cli/src/main.rs:1739-1832`).

**Phase 2 — the new-code fault kinds.**
- `stuck_at_bit`: a `stuck_mask` honoured in `GenericPeripheral::write*`
  (`declarative.rs:214,297`).
- `bound_violation` / `permission_violation` / `memory_corruption`: a synthetic
  fault-window + per-tick corruption service at the bus accessor + service-loop
  chokepoint (`accessors.rs:192`; pattern from `service_hcsr04`, `bus/mod.rs:778`).

**Phase 3 — HardFault vectoring (CPU task; unblocks `handler_ran` verdicts).**
- Make a data/fetch `MemoryViolation` optionally escalate to ARMv7-M HardFault
  (exc 3) by pending it through the existing exception-dispatch path
  (`cpu/cortex_m.rs:700-805`) instead of returning `Err`. Gated/opt-in so existing
  configs that rely on abort-on-violation (e.g. the bench `rambug`,
  `rambug-smoke.yaml`) are unchanged. This is the one genuinely CPU-side piece and
  is correctly deferred to its own phase. Only with it can the verdict vocabulary
  add `handler_ran` (firmware's HardFault_Handler observed to run + write the
  §2.5 verdict word).

**Phase 4 — upstream a slim in-repo fault-bench example.**
- Add `examples/fault-injection-bench/` reusing ONE clean firmware (no per-bug
  variants) + a `faults.yaml` test that injects `missing_clock`,
  `wrong_reset_value`, `delayed_irq`, `bound_violation` against it and asserts
  `verdict.safe_when`. This is the f103-fidelity-bench story inverted: same chip,
  fixed firmware, perturbed environment — the cert-correct framing. Keep it slim
  (one firmware, ~4 fault cases) and wire it into `run-benchmark.sh`-style CI.

## 7. Risks

- **Silent no-op faults** — the #1 risk; mitigated by the two-layer fire gate
  (§5). Treat `require_fault_fired: true` as the default and document loudly.
- **Data-violation swallowing (§2.1).** Until Phase 3, `bound`/`permission`
  faults only manifest as a sim abort or an absent marker, not a handler run.
  Don't over-promise `handler_ran` verdicts pre-Phase-3.
- **Byte-wise write triggers.** `GenericPeripheral` writes byte-by-byte and
  multi-byte write-triggers are already flagged as a limitation
  (`declarative.rs:243-251`). `on_write` fault triggers with a 32-bit `value`
  must use the u32 write path (`declarative.rs:340`) — document/validate.
- **Schema-version churn.** `TestScript` is `deny_unknown_fields`
  (`config/src/lib.rs:652`) and `validate` hard-pins `"1.0"` (`:672`); the bump to
  `"1.1"` must keep `1.0` scripts passing. Covered by Phase-0 round-trip tests.
- **Evidence-contract drift with #2** — mitigated by locking `FaultEvidence` in
  Phase 0, not at the end.
- **Plausibility creep.** Resist adding fantasy faults; every kind in §4.3 maps to
  a documented silicon failure mode. Keep the guardrail compiler strict.

## 8. Composition recap
- **#1:** every peripheral/timing fault is a `GenericPeripheral`
  `TimingDescriptor`/`SideEffectsDescriptor`/reset/access mutation — strictly on
  #1's substrate, no parallel system.
- **#2:** the per-fault `FaultEvidence` (`fault_fired` + `verdict` + resolved
  target, §4.6) is the exact record #2's signable cert report ingests; shape
  locked in Phase 0.
