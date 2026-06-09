# nRF52840 Silicon Conformance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring nRF52840 onboarding to STM32F103 parity — every notable peripheral swept against real silicon, promoted into the production chip descriptor with verified reset values, and locked behind a ratcheted sim-vs-hardware conformance gate.

**Architecture:** Drive the existing nRF52 register sweeps against a live Seeed XIAO nRF52840 Sense over ST-LINK + OpenOCD. Promote the peripherals that match silicon into `configs/chips/nrf52840.yaml` with provenance. Add a behavioral-digest conformance firmware + diff test mirroring `firmware-f103-conformance` / `f103_conformance.rs`. Resolve the one WDT divergence honestly (fix model or document as residual). Back the `verified` flag with the gate.

**Tech Stack:** Rust (workspace crates), `thumbv7em-none-eabi` firmware, OpenOCD 0.12.0 + `interface/stlink.cfg` + `target/nrf52.cfg`, YAML chip/system descriptors.

**Grounded verdict table (run 2026-06-09 against live silicon):**
- `nrf52_mmio_diff` (production 4: UART0, SPIM0, GPIO0/1): **16/16 match**.
- `nrf52_onboarding_diff` (22 peripherals): **21 MODELLED**, 1 `WRONG_LAYOUT` = WDT (CRV: sim `0x00020000` vs hw `0xFFFFFFFF`).

**Commands reference (hardware-in-the-loop):**
```bash
cd ~/projects/labwired/core
# onboarding sweep (22 periph):
cargo test -p labwired-hw-oracle --test nrf52_onboarding_diff --features hw-oracle-nrf52 -- --ignored --nocapture
# production sweep (4 periph):
cargo test -p labwired-hw-oracle --test nrf52_mmio_diff --features hw-oracle-nrf52 -- --ignored --nocapture
```

---

### Task 1: Resolve the WDT CRV divergence (the one WRONG_LAYOUT)

**Files:**
- Inspect: `core/crates/core/src/peripherals/nrf52/wdt.rs`
- Inspect: `core/crates/hw-oracle/tests/nrf52_onboarding_diff.rs` (the WDT case + `expect`)
- Modify: whichever of the two the investigation indicts
- Docs: `core/examples/nrf52840/VALIDATION.md` (created in Task 7) records the verdict

- [ ] **Step 1: Reproduce + characterize on silicon.** Run a targeted OpenOCD read to see CRV's true behavior under reset-halt vs after explicit WDT stop:
```bash
cd ~/projects/labwired/core
timeout 30 openocd -f interface/stlink.cfg -f target/nrf52.cfg \
  -c init -c "reset halt" \
  -c "echo {WDT RUNSTATUS / CRV at reset}" \
  -c "mdw 0x40010400 1" \
  -c "mdw 0x40010504 1" \
  -c "mww 0x40010504 0x00020000" \
  -c "mdw 0x40010504 1" \
  -c shutdown 2>&1 | grep -E '0x40010'
```
Expected to confirm: CRV (`0x40010504`) reads `0xFFFFFFFF` at reset and the write does not stick because RUNSTATUS (`0x40010400`) shows the WDT already running (bootloader-started). If RUNSTATUS=0 and the write DOES stick, the root cause is instead a sim model bug.

- [ ] **Step 2: Decide + apply.**
  - **If WDT is running on silicon (environmental):** this is a faithful divergence, not a model bug. Correct the test `expect` for the WDT/CRV case to `0xFFFFFFFF` (locked-while-running) OR have the sweep stop the WDT first to match. Do NOT hack the sim to fake `0xFFFFFFFF` unconditionally. Document as a known residual with cause.
  - **If WDT is stopped on silicon (model bug):** the sim model wrongly persists a CRV write that silicon rejects/reset-masks. Fix `wdt.rs` so CRV reset value is `0xFFFFFFFF` and write semantics match silicon. Re-sweep.

- [ ] **Step 3: Re-run the onboarding sweep, confirm WDT verdict resolves.**
```bash
cargo test -p labwired-hw-oracle --test nrf52_onboarding_diff --features hw-oracle-nrf52 -- --ignored --nocapture 2>&1 | grep -E 'WDT|verdict|test result'
```
Expected: WDT line `[OK]` or, if residual, the case documented and the `NRF52_STRICT`-off run still green.

- [ ] **Step 4: Commit.**
```bash
git add -A && git commit -m "fix(nrf52): reconcile WDT CRV behavior with silicon"
```

> NOTE for executor: Task 1 is the single peripheral needing model work. If subagent-driven, dispatch ONE Haiku subagent with the silicon readback from Step 1 and the file paths above. All other peripherals already match silicon.

---

### Task 2: `hw-capture-nrf52840.sh` ground-truth capture script

**Files:**
- Create: `core/scripts/hw-capture-nrf52840.sh`
- Reference template: `core/scripts/hw-capture-stm32f103.sh`

- [ ] **Step 1: Clone the F103 script structure**, retargeted to nRF52. Replace the ST-LINK/stm32f1x invocation with `-f interface/stlink.cfg -f target/nrf52.cfg`. Dump FICR identity and the notable reset registers for every promoted peripheral. Use this register list (silicon addresses, NOT the sim P1 remap):
```
# Identity
FICR INFO.PART      0x10000100
FICR INFO.VARIANT   0x10000104
FICR DEVICEID[0..1] 0x10000060 (2 words)
# Cortex-M4 system
SCB VTOR            0xE000ED08
SysTick CTRL        0xE000E010
# Peripheral reset-state (one+ notable reg each)
TIMER0 BITMODE      0x40008508 ; PRESCALER 0x40008510
RTC0   PRESCALER    0x40011508
WDT    CRV          0x40010504 ; RUNSTATUS 0x40010400
RNG    CONFIG       0x4000D504
PWM0   COUNTERTOP   0x4001C548
SAADC  RESOLUTION   0x40007510
QSPI   IFCONFIG0    0x40029544
COMP   TH           0x40013530
QDEC   SAMPLEPER    0x40012508
PDM    PDMCLKCTRL   0x4001D540
GPIO0  OUT          0x50000504 ; DIR 0x50000514
GPIO1  OUT          0x50000804 ; DIR 0x50000814   # silicon base 0x50000300+? use real P1 base
UART0  ENABLE       0x40002500
SPIM0  ENABLE       0x40003500
NVMC   READY        0x4001E400
USBD   ENABLE       0x40027500
RADIO  FREQUENCY    0x40001508 ; MODE 0x40001510
```
> NOTE: confirm GPIO1 silicon base — datasheet P1 is `0x50000300` window; the sim remaps to `0x50001000`. The capture script must use the **silicon** address.

- [ ] **Step 2: Make executable, run against the board.**
```bash
chmod +x core/scripts/hw-capture-nrf52840.sh
core/scripts/hw-capture-nrf52840.sh
ls core/fixtures/nrf52840/hw-capture-*/
```
Expected: a timestamped dir with `st-info.txt`, `registers.txt` showing `INFO.PART=00052840`.

- [ ] **Step 3: Commit, split across the public/private boundary.** The repo split IS the moat boundary: `core/` = public `labwired-core` (code), the superproject root = private `labwired` (validation evidence). So:
  - **Capture script → public `core`** (it's tooling). `core/fixtures/` is gitignored (`core/.gitignore:8`), so the script's scratch output never pollutes the public repo.
  - **Captured silicon traces → private superproject** at `validation/silicon/nrf52840/hw-capture-<ts>/` (curate the one complete run out of the gitignored scratch; add a README documenting provenance).
```bash
# public submodule — script only
git -C core add scripts/hw-capture-nrf52840.sh
git -C core commit -m "feat(nrf52): hw-capture-nrf52840.sh reset-state truth-set capture"
# private superproject — the corpus
cp -r core/fixtures/nrf52840/hw-capture-<ts> validation/silicon/nrf52840/
git add validation/silicon/nrf52840/
git commit -m "validation: nRF52840 silicon reset-state corpus (capture <ts>)"
```

---

### Task 3: Promote verified peripherals into `configs/chips/nrf52840.yaml`

**Files:**
- Modify: `core/configs/chips/nrf52840.yaml` (currently 4 peripherals → full verified set)
- Reference style: `core/configs/chips/stm32f103.yaml:79-133` (reset-value + provenance comments)
- Source of truth: the captured `registers.txt` from Task 2 and the green sweep

- [ ] **Step 1: Add each MODELLED peripheral** (TIMER0-4, RTC0-2, RNG, PPI, PDM, GPIOTE, ECB, TEMP, SAADC, PWM0-3, QSPI, NFCT, COMP, QDEC, EGU0-5, FICR, NVMC, USBD, ACL, CRYPTOCELL, RADIO + WDT once Task 1 green) with a `profile`/reset-value block and a provenance comment per peripheral:
```yaml
  # verified against nRF52840 (Seeed XIAO Sense), ST-LINK V2, 2026-06-09
  timer0:
    type: nrf52840_timer
    base_address: 0x40008000
    irq: 8
    config: { profile: "nrf52" }
    reset_values:
      BITMODE: 0x00000000     # 16-bit at reset (silicon-confirmed)
      PRESCALER: 0x00000004
```
Use the **silicon** addresses; do not copy the sim P1 remap into this file.

- [ ] **Step 2: Re-run both sweeps to confirm promotion didn't regress.**
```bash
cargo test -p labwired-hw-oracle --test nrf52_onboarding_diff --features hw-oracle-nrf52 -- --ignored --nocapture 2>&1 | grep -E 'verdict|test result'
cargo test -p labwired-hw-oracle --test nrf52_mmio_diff --features hw-oracle-nrf52 -- --ignored --nocapture 2>&1 | grep -E 'summary|test result'
```
Expected: onboarding all `MODELLED` (WDT resolved), mmio `match=16`.

- [ ] **Step 3: Validate the chip config loads** in a non-HW build (catches YAML/schema errors):
```bash
cargo test -p labwired-core --test '*' chip 2>&1 | tail -20   # or the repo's config-load test
```

- [ ] **Step 4: Commit.**
```bash
git add core/configs/chips/nrf52840.yaml
git commit -m "feat(nrf52): promote silicon-verified peripherals into chip descriptor"
```

---

### Task 4: `firmware-nrf52840-conformance` crate

**Files:**
- Create: `core/crates/firmware-nrf52840-conformance/Cargo.toml`
- Create: `core/crates/firmware-nrf52840-conformance/src/main.rs`
- Create: `core/crates/firmware-nrf52840-conformance/.cargo/config.toml` (target `thumbv7em-none-eabi`)
- Create: `core/crates/firmware-nrf52840-conformance/memory.x` / linker (copy from `firmware-nrf52840-demo`)
- Reference: `core/crates/firmware-f103-conformance/src/main.rs`

- [ ] **Step 1: Scaffold the crate** mirroring `firmware-nrf52840-demo`'s Cargo.toml + `.cargo/config.toml` + linker. Add to workspace members if the workspace uses an explicit list.

- [ ] **Step 2: Write the digest firmware.** Define a verdict block at a fixed RAM address (mirror F103's `0x2000_3000`, validate it is in nRF52840 RAM `0x2000_0000..0x2004_0000`). Exercise **deterministic** peripherals and write one digest word each, ending with `DONE_MAGIC`:
```rust
#![no_std]
#![no_main]
// digest layout (DONE first, then per-peripheral words):
//  [0] DONE magic   [1] gpio_out   [2] timer0_cc_capture
//  [3] rtc0_counter [4] ecb_ciphertext_word0  [5] gpiote_ppi_event
//  [6] temp_in_range (bool)  [7] rng_valrdy_fired (bool, liveness only)
const VERDICT_ADDR: usize = 0x2000_3000;
const DONE_MAGIC: u32 = 0x52840D0E;
// ECB: fixed key + plaintext -> deterministic ciphertext; digest word0 of result.
// RNG: assert VALRDY fired; DO NOT digest the random value.
```
Drive each peripheral via raw MMIO (no HAL dependency, like the F103 conformance firmware), then store digest words and finally `DONE_MAGIC`.

- [ ] **Step 3: Build it.**
```bash
cargo build --release -p firmware-nrf52840-conformance --target thumbv7em-none-eabi
```
Expected: ELF at `core/target/thumbv7em-none-eabi/release/firmware-nrf52840-conformance`.

- [ ] **Step 4: Commit.**
```bash
git add core/crates/firmware-nrf52840-conformance
git commit -m "feat(nrf52): behavioral-digest conformance firmware"
```

---

### Task 5: `nrf52_conformance.rs` sim-vs-hardware diff test

**Files:**
- Create: `core/crates/hw-oracle/tests/nrf52_conformance.rs`
- Reference: `core/crates/hw-oracle/tests/f103_conformance.rs`
- Feature gate: `hw-oracle-nrf52` (HW path), sim path always runs

- [ ] **Step 1: Sim path** — load the conformance ELF onto the nrf52 sim bus, step until `DONE_MAGIC` appears at `VERDICT_ADDR`, read the digest block. Mirror `f103_conformance.rs:62-119` including spot-check `assert_eq!` on the deterministic words (gpio_out, ecb_ciphertext_word0).

- [ ] **Step 2: HW path** (`#[cfg(feature = "hw-oracle-nrf52")]`) — `OpenOcd::spawn_nrf52()`, flash the conformance firmware to the **application region** (NOT mass-erase — preserve the UF2 bootloader; set VTOR to the app base), poll `DONE_MAGIC` for up to 5 s, halt, read the digest. Mirror `f103_conformance.rs:126-164`.

- [ ] **Step 3: Diff + ratchet.** Compare digest words (masked), report per-field `sim 0xXXXX vs hw 0xYYYY`, set `const BASELINE_MATCHED` to the count actually achieved on first green run, and document any residual inline (the way F103 documents EXTI re-pend). RNG word is liveness-only — assert "fired", not value-equal.

- [ ] **Step 4: Run sim path (no HW feature) — must pass in plain CI.**
```bash
cargo test -p labwired-hw-oracle --test nrf52_conformance 2>&1 | tail -20
```
Expected: PASS (sim spot-checks).

- [ ] **Step 5: Run full sim-vs-HW path against the board.**
```bash
cargo test -p labwired-hw-oracle --test nrf52_conformance --features hw-oracle-nrf52 -- --ignored --nocapture 2>&1 | tail -30
```
Expected: digest matches at the ratcheted baseline; residuals documented.

- [ ] **Step 6: Commit.**
```bash
git add core/crates/hw-oracle/tests/nrf52_conformance.rs core/crates/hw-oracle/Cargo.toml
git commit -m "feat(nrf52): sim-vs-silicon behavioral conformance gate"
```

---

### Task 6: Documentation — fidelity table + audit trail

**Files:**
- Modify: `core/docs/boards/nrf52840.md` (per-peripheral fidelity table)
- Create: `core/examples/nrf52840/VALIDATION.md` (per-round audit, `nucleo-l476rg/VALIDATION.md` style)

- [ ] **Step 1: Fidelity table** in `nrf52840.md` — one row per peripheral: Modeled / Verified-against-silicon / Residual, with the 2026-06-09 sweep as the evidence date.

- [ ] **Step 2: `VALIDATION.md`** — record: production-4 = 16/16 match; onboarding-22 = 21 MODELLED + WDT resolution (from Task 1); conformance baseline (from Task 5); the UF2-bootloader flashing decision; the P1 sim-remap note.

- [ ] **Step 3: Commit.**
```bash
git add core/docs/boards/nrf52840.md core/examples/nrf52840/VALIDATION.md
git commit -m "docs(nrf52): fidelity table + silicon validation audit trail"
```

---

### Task 7: Back the `verified` flag with the gate + final clean re-run

**Files:**
- Modify: `core/configs/onboarding/nrf52840.yaml` (`verified` / `pass_rate` / validation method)
- Reference: `core/configs/onboarding/stm32f103.yaml` (`validation: ci-arm-fixture-simulation`)

- [ ] **Step 1: Update the onboarding metadata** so `verified`/`pass_rate`/`validation` reference the new conformance gate (e.g. `validation: hil-nrf52-conformance`), not an unsubstantiated flag.

- [ ] **Step 2: Final clean re-run of all three HW sweeps.**
```bash
cd ~/projects/labwired/core
for t in nrf52_onboarding_diff nrf52_mmio_diff nrf52_conformance; do
  echo "== $t =="
  cargo test -p labwired-hw-oracle --test $t --features hw-oracle-nrf52 -- --ignored --nocapture 2>&1 | grep -E 'verdict|summary|test result|DIFF|diverge'
done
```
Expected: onboarding all MODELLED (or documented residual), mmio match=16, conformance at baseline.

- [ ] **Step 3: Run the non-HW test suite to confirm nothing regressed for plain CI.**
```bash
cargo test -p labwired-core 2>&1 | tail -15
cargo test -p labwired-hw-oracle --test nrf52_conformance 2>&1 | tail -10
```

- [ ] **Step 4: Commit.**
```bash
git add core/configs/onboarding/nrf52840.yaml
git commit -m "feat(nrf52): back verified flag with silicon conformance gate"
```

---

## Self-Review notes

- **Spec coverage:** D1→Task 2, D2→Task 3, D3→Tasks 4+5, D4→Tasks 6+7, WDT iteration loop→Task 1. All four deliverables + the Haiku iteration loop are covered.
- **UF2 bootloader risk:** handled explicitly in Task 5 Step 2 (flash to app region, no mass-erase).
- **P1 remap risk:** handled in Task 2 Step 1 and Task 3 Step 1 (use silicon addresses, not sim remap).
- **Non-deterministic peripherals:** RNG is liveness-only in Tasks 4/5 (no value digest).
- **No-tamper discipline:** Task 1 Step 2 forbids faking the WDT value to force green; resolution must be faithful (model fix or documented residual).
- **Addresses marked for confirmation:** GPIO1/P1 silicon base in Task 2/3 — executor must verify against the datasheet before committing reset values.
