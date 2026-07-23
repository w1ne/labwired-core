# LabWired test harness

How tests are layered, what “green” means, and where new tests go.

This is the contract for harness work started in `chore/test-harness-organize`
(2026-07-23). Product matrices are **smoke**, not full silicon proof.

---

## Layers

```text
L3  Product matrices     validation/{arduino,zephyr}-matrix/
    Real stock FW + labwired test. Stage-classified pass/fail.

L2  labwired test        crates/cli (test subcommand)
    Script → build machine → advance → assertions → result.json

L1  Core tests           crates/core/{src,tests}
    Units next to models; differentials; e2e; diag (never CI)
```

**Rule:** L3 must not special-case silicon. Chip bring-up belongs in
`system` / `boot` / models — not in matrix Python or growing `if is_esp32s3`
trees in the CLI (extraction is phased; see roadmap below).

---

## What “pass” means

| Layer | Pass means | Not a pass if… |
|-------|------------|----------------|
| **Unit** (`--lib`) | Rust assertions hold | model math/API wrong |
| **Differential** | Walk vs scheduler (or tick-1 vs N) traces match | time-base drift, IRQ mismatch |
| **E2E** (`crates/core/tests/e2e_*`) | Specific lab/fixture contract | that demo broken |
| **Matrix cell** | Staged: compile → boot → **oracle** | any stage fails |
| **Diag** | N/A — manual only | never gate CI |

### Matrix oracle (current + target)

| Level | Current | Target |
|-------|---------|--------|
| L0 | UART contains marker | same (boot smoke) |
| L1 | UART contains marker after loop/delay | same + marker from scheduling path |
| L2 | UART contains marker | UART **and** (when configured) **LED GPIO edges** via `--watch-gpio` |

Matrix is **not** a substitute for walk differentials or hw-oracle.

### Failure buckets (matrix)

| Status | Meaning |
|--------|---------|
| `pass` | Oracle satisfied |
| `compile_fail` / `build_fail` | Toolchain rejected the sketch/sample |
| `toolchain_missing` | Platform/board package or west missing |
| `boot_fail` | Sim ran; empty UART / no progress toward marker |
| `oracle_fail` | UART (or GPIO) ran but assertion missed |
| `unmodeled` | Gap signal in result/stop_reason or known fault strings |
| `timeout` | Wall clock budget |
| `sim_error` | Hard sim failure |

Classification prefers `result.json` `status` / `stop_reason` over grepping stderr.

---

## Env vars (matrix / test)

| Var | Role |
|-----|------|
| `LABWIRED_MATRIX_SPEED=1` | Opt-in idle fast-forward in `labwired test` (**requires** CLI built with `--features event-scheduler`). Does **not** widen tick interval. Experimental; ESP FreeRTOS labs may fail under event-scheduler — default CLI is the fidelity path. |
| `LABWIRED_RP2040_BOOTROM` | Path to RP2040 mask ROM; matrix sets default when in-tree image exists. Empty env can opt out for bare-metal tests. |
| `LABWIRED_RISCV_JIT` / `LABWIRED_TICK_INTERVAL` | C3 JIT / tick experiments (not matrix default). |

Default `cargo build -p labwired-cli --release` does **not** enable `event-scheduler`.

---

## Where to put a new test

| You are proving… | Put it here |
|------------------|-------------|
| One peripheral / CPU helper | Unit next to the model in `crates/core/src/...` |
| Walk-delete / idle-FF / tick identity | `crates/core/tests/*differential*` (prefer always-on or small fixture; full-boot diffs may `#[ignore]`) |
| A shipped lab / multi-peripheral story | `crates/core/tests/e2e_*.rs` |
| Temporary “what is PC doing” archaeology | `diag_*.rs` with **`#[ignore]`** — never CI |
| Stock Arduino/Zephyr still boots on a chip | Matrix board row + sketch/sample |

**Do not** add new hardcoded PlatformIO work paths to `crates/cli`.

---

## Integration test inventory (`crates/core/tests/`)

Counts as of 2026-07-23 (~109 `*.rs` files). Buckets are **logical** (naming);
physical rehome is a later PR.

| Bucket | Count | Policy |
|--------|------:|--------|
| `diag_*` | 6 | Always `#[ignore]`; never CI |
| `*differential*` / walk-free | 19 | Many need `event-scheduler`; some full-boot ignored |
| `e2e_*` / survival / shipped-lab | 17 | Mix of always-on and fixture/feature gated |
| `*_profile` / bench | 2 | `#[ignore]` + release |
| other | 65 | Conformance, world, JIT lockstep, onboarding, … |

### diag (manual only)

| Test | ignored | features |
|------|---------|----------|
| `diag_c3_yield` | yes | — |
| `diag_esp32_dual_core_boot` | yes | — |
| `diag_esp32c3_boot` | yes | — |
| `diag_esp32c3_panic` | yes | — |
| `diag_esp32s3_boot` | yes | — |
| `diag_esp32s3_l2` | yes | — |

### differential (representative)

Prefer small fixtures in PR CI. Full ROM+app walk diffs stay `--ignored` until cheaper.

Many files use `#![cfg(feature = "event-scheduler")]`. Default CI does **not** prove
scheduler identity unless a workflow enables the feature.

### Product matrices

| Matrix | Driver | Shared engine |
|--------|--------|----------------|
| Arduino L0–L2 | `validation/arduino-matrix/run_matrix.py` | `validation/matrix_lib/` |
| Zephyr L0–L2 | `validation/zephyr-matrix/run_matrix.py` | `validation/matrix_lib/` |

---

## Roadmap (harness reorg)

| Phase | Status | Notes |
|-------|--------|-------|
| **A** Doc + inventory | **done** (this file) | Contract freeze |
| **B** `matrix_lib` + `--sim-only` + ELF hash cache | **done** | Local branch only |
| **C** L2 GPIO oracle + budget reasons | **done** | `led_watch` + ratcheted `max_steps` |
| **D** Structured fail classify | partial | `matrix_lib.classify_failure` prefers result.json |
| **E** Extract ESP bring-up from CLI | later | Highest risk; re-run 45/45 |
| **F** Physical test rehome | later | `tests/{diag,differential,e2e}/` |
| **G** Dual-universe nightly (Cortex) | later | event-scheduler smoke |

### Honest perf notes (2026-07-23)

- Dual-core `is_parked_idle` primary batch is **live** when APP is WAITI-parked:
  plan allows multi-instruction PRO windows even at `tick_interval=1`, and
  commit coalesces peripheral `tick_elapsed(N)`.
- SCB presence still forces quantum-1 (cycle-accurate logic capture). RTC_CNTL
  only forces quantum-1 while SW_SYS_RST is latched.
- Dual-universe (`event-scheduler` + `MATRIX_SPEED=1`): green on Class-M and
  **ESP32-S3**; classic ESP empty UART; C3 L2 may hang after `LW_L2_BOOT`.
  Smoke script: `validation/arduino-matrix/scripts/matrix_speed_subset.sh`.
  Default CLI remains the full-matrix fidelity path.
- CLI must not hard-code matrix PIO work paths for partitions.
- L2 oracles: GPIO/`sio` edges via `led_watch` (STM32, ESP classic, nRF P0.17,
  RP2040 SIO:25); C3 `rmt:0` synthetic TX edges + `min_rmt_tx` (single
  `esp32c3_rmt` entry — no declarative+behavioral dual stack); S3 `min_rmt_tx`
  inspect count.

Work log: [`test_harness_reorg_log.md`](test_harness_reorg_log.md).

---

## Related docs

- `validation/arduino-matrix/README.md` / `PROBLEMS.md`
- `validation/zephyr-matrix/README.md`
- `docs/walk_free_plan.md` — walk vs scheduler strategy
- `docs/coverage_scoreboard.md` — coverage matrix (separate from Arduino matrix)
