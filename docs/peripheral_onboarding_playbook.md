# Peripheral Onboarding Playbook (universal)

Reusable, **chip-agnostic** procedure for onboarding an individual peripheral and
validating it against real silicon. The same loop — **capture → model → oracle →
validate** — brought up STM32H563, nRF52840, STM32F407/F103, and the ESP32 parts.
Only the *capture front-end* differs per architecture (Appendix A); everything
else is identical regardless of chip.

Read alongside the architectural contracts — this is the *process*, those are the
*rules*:

- [`docs/peripheral_development.md`](peripheral_development.md) — the `Peripheral` trait, byte-granular register access, the `tick()` time base.
- [`docs/CONTRIBUTING_PERIPHERALS.md`](CONTRIBUTING_PERIPHERALS.md) — determinism / fidelity / test-rigor standards.
- [`docs/board_onboarding_playbook.md`](board_onboarding_playbook.md) — whole-*board* bring-up (chip yaml + system manifest + smoke firmware).
- [`docs/hardware_interaction_guide.md`](hardware_interaction_guide.md) — DMA/IRQ propagation via the two-phase heartbeat tick.

> **Golden rule:** a peripheral is "onboarded" only when its modeled register
> behavior has been diffed against real silicon and any divergence is understood.
> Code that compiles and passes a hand-written unit test is *modeled*, not
> *validated*. That difference is the entire point of LabWired.

---

## 0. Two modeling paths — pick before you start

Independent of chip, every peripheral is wired one of two ways. Choose by whether
the block has *behavior* (state machines, IRQs, DMA, timing) or is mostly a
*register file*.

| Path | When | Where | Wiring |
|------|------|-------|--------|
| **Declarative YAML** | Register-file blocks: reset values, RAZ/WI, simple W1C, field reads. | `configs/peripherals/<chip>/<periph>.yaml` | `type: declarative`, `config:` entry in `configs/chips/<chip>.yaml`. Generate boilerplate with `svd-ingestor`. |
| **Rust model** | Behavioral blocks: timers, UART/I²C/SPI engines, DMA, PWM, crypto — anything with `tick()` logic, IRQs, or cross-peripheral side-effects. | shared `crates/core/src/peripherals/<periph>.rs`, or chip-specific `crates/core/src/peripherals/<chip>/<periph>.rs` | YAML-declared base + dynamic load (`crates/core/src/bus/mod.rs`), **or** programmatic instantiation in the chip's `system/*` builder. |

Two registration styles exist and are chip/arch-dependent — both are first-class:
- **Declarative / dynamic** (e.g. ESP32-C3 RISC-V, generic Cortex-M): chip yaml
  declares base + type; the bus loads and wires it.
- **Programmatic** (e.g. ESP32-S3 Xtensa): a `configure_*()` builder in
  `crates/core/src/system/` instantiates peripherals at TRM base addresses.

Per-chip current state lives in `docs/boards/<board>.md` (fidelity tables).

---

## 1. Prerequisites (per peripheral)

1. **MCU Reference Manual / TRM** — register tables, reset values, field semantics.
2. **Datasheet** — memory-map boundaries, peripheral base addresses.
3. **SVD** (vendor CMSIS pack or Espressif `svd/`) — feed to `svd-ingestor` for
   declarative boilerplate.
4. **Probe firmware** that exercises *only* the target peripheral (one block at a
   time keeps the oracle diff legible).
5. **Capture front-end for the target architecture** — see Appendix A.

---

## 2. The loop (identical for every chip)

Reference scaffolding lives in `scripts/hw-oracle/` and the replay crate
`crates/hw-oracle`. The L476 path (`crates/core/tests/firmware_survival.rs`) is the
canonical "done right" example: six survival tests asserting byte-for-byte UART
parity with silicon across UART/SPI/I²C/ADC/DMA.

### Step 1 — Capture from real silicon
Flash the single-peripheral probe firmware, then record a baseline trace. Captures
follow the convention `scripts/hw-oracle/captures/<chip>/<utc-ts>/`:

| File | Contents |
|------|----------|
| `oracle.json` | manifest: chip, sample params, checkpoint region map |
| `pc_trace.tsv` | `step  ns_offset  pc_hex` — PC sampled at fixed wall-clock intervals (halt→read PC→resume) |
| `mem_pre.json` / `mem_post.json` | `{addr: word}` snapshots of checkpoint windows before/after the run |
| `<probe>.log` | raw debugger output |
| `elf.path` | absolute path of the flashed firmware |

For a new peripheral, **add its MMIO window (and any RAM it touches) to the
checkpoint regions** so the diff covers it. What's sampleable without a logic
analyzer: PC traces + periodic MMIO/RAM snapshots. Bus-wire snooping
(SPI/I²C line traffic) is out of scope — validate those at the register/FIFO
level, not the wire level.

### Step 2 — Model the peripheral
Implement per §0 and `peripheral_development.md`:
- Byte-granular `read`/`write` reconstructing 32-bit registers (`offset & !3`).
- Side-effects (W1C, FIFO pops, start bits) in `write`.
- Time-based behavior in `tick()` — **deterministic only**: no wall-clock, no
  `thread::sleep`, no RNG; decimate high-frequency logic with a cycle counter.
- Emit IRQs / DMA / cross-peripheral writes via `PeripheralTickResult`
  (`irq`, `explicit_irqs`, `dma_requests`, `dma_signals`, `mmio_writes`, `cycles`).

### Step 3 — Oracle diff (MMIO)
The "oracle" is the silicon baseline. Replay the same ELF in-sim and diff:

```bash
cargo run --release -p labwired-hw-oracle --bin <chip>_replay_in_sim -- \
  --capture scripts/hw-oracle/captures/<chip>/<ts> \
  --elf <same-firmware.elf>
```

It reports the **first divergence point** (PC or checkpoint word). Each divergence
is a real model bug — wrong reset value, missing field, mis-timed flag, absent
side-effect. Fix, re-run, repeat until clean (or the residual is documented).

### Step 4 — Lock it in with a survival/parity test
Mirror `crates/core/tests/firmware_survival.rs`: assert byte-for-byte parity on the
validated behavior (UART output, register sequence, IRQ count). This is what the CI
gates run and what protects the model from regression.

---

## 3. Wiring checklist

### Declarative / dynamic chips (e.g. C3, generic Cortex-M)
1. `configs/peripherals/<chip>/<periph>.yaml` — generate with `svd-ingestor`, then hand-tune reset values/fields against the TRM.
2. Entry in `configs/chips/<chip>.yaml`:
   ```yaml
   - id: "<periph>"
     type: "declarative"          # or a behavioral type name for a Rust model
     base_address: 0xXXXXXXXX
     config: "<periph>.yaml"
     irq: <n>                      # if it raises interrupts
   ```
3. If behavioral, register the Rust type in the dynamic load path (`crates/core/src/bus/mod.rs`).

### Programmatic chips (e.g. S3 Xtensa)
1. `crates/core/src/peripherals/<chip>/<periph>.rs` implementing `Peripheral`.
2. `mod <periph>;` in the chip's `peripherals/<chip>/mod.rs`.
3. Instantiate + map in `crates/core/src/system/<arch>.rs::configure_*()` at the TRM base.

---

## 4. Blob-driven blocks (radio: WiFi / BLE / 802.15.4)

Radio MAC/PHY and BLE link-layers are firmware-blob-driven. Two valid strategies —
**decide and record the choice per chip** in that chip's fidelity table:

- **Register model** — model the controller registers and validate against silicon
  captures like any other peripheral. Highest fidelity; largest effort; some PHY
  behavior is only validatable with RF capture gear, so document what's covered vs.
  asserted. Follow §2 exactly.
- **Thunk + SimNet** (legacy pattern, `crates/core/src/peripherals/esp32s3/wifi_thunks.rs`,
  `rom_thunks.rs`) — intercept the API/ROM call at its PC, short-circuit to a
  deterministic outcome, route the data plane through the in-sim `SimNet`. Validate
  end-to-end (a real sketch completing an HTTP/BLE transaction in-sim), not
  register-by-register.

Whichever path, the deliverable is the same: deterministic, replayable behavior +
a committed test.

---

## 5. Definition of done (CI gates)

1. **Builds** for the target(s); `cargo fmt` / `clippy` clean.
2. **Survival/parity test** added (§2 Step 4) and green.
3. **Oracle diff** against a committed capture is clean (or residual documented in
   the peripheral's `VALIDATION.md`).
4. CI green — the relevant subset of:
   - `core-ci.yml` — build/test/clippy.
   - `core-onboarding-smoke.yml` — fires on `configs/chips/`, `examples/`, firmware-crate changes.
   - `core-board-ci-fixture-arm.yml` / `core-board-ci-fixture-riscv.yml` — per-arch smoke + instruction-support coverage.
   - `core-unsupported-audit.yml` — no unsupported instructions in hot paths.
   - `core-validate-hw-targets.yml` — onboarding pass-rate manifest (`configs/onboarding/*.yaml`).
5. **`examples/<board>/` updated**: README, `VALIDATION.md` (bug-discovery audit
   trail), the probe firmware/script used.

---

## 6. Per-peripheral quick checklist

```
[ ] TRM register table + reset values transcribed
[ ] Path chosen (declarative YAML vs Rust model)         §0
[ ] Probe firmware exercises ONLY this peripheral
[ ] Captured from real silicon                           §2.1 / Appendix A
[ ] Modeled (byte-granular regs, deterministic tick)     §2.2
[ ] Wired into chip (yaml / system builder)              §3
[ ] Oracle diff clean (or divergence documented)         §2.3
[ ] Survival/parity test added                           §2.4
[ ] examples/<board>/VALIDATION.md updated               §5
[ ] CI green                                             §5
```

---

## Appendix A — Capture front-ends per architecture

The loop is identical; only how you halt the core, read PC, and dump memory
differs. Parameterize the capture script with the right probe/target configs.

| Architecture | Examples | Probe / transport | Notes |
|--------------|----------|-------------------|-------|
| **Cortex-M (ARM)** | STM32F1/F4/H5, RP2040, nRF52840 | ST-Link / CMSIS-DAP via OpenOCD SWD; `st-flash` | `scripts/hw-capture-stm32*.sh`, `hw-capture-nrf52840.sh`. macOS: ST-Link works without timeout flags; mind connect-under-reset (`st-flash --connect-under-reset`). |
| **RISC-V** | ESP32-C3 (RV32IMC) | Built-in **USB-Serial/JTAG** (`303a:1001`) via OpenOCD `board/esp32c3-builtin.cfg` | No external probe. Distro OpenOCD 0.12.0 **lacks** esp32c3 targets — use Espressif's `openocd-esp32` fork. |
| **Xtensa** | ESP32-S3 (LX7), ESP32-WROOM (LX6) | S3/C-series: built-in USB-JTAG. WROOM: ESP-Prog (FT2232H) | `scripts/hw-oracle/esp32_capture.sh` (parameterized via `ESP32_OPENOCD_IF` / `ESP32_OPENOCD_TGT`); `README_esp32.md`. Decoder cross-check vs `xtensa-esp-elf-objdump` (`b7/b8-sweep.sh`). |

**Identifying ESP boards** (C3 and S3 both enumerate as `303a:1001`):
```bash
esptool.py --port /dev/cu.usbmodemXXXX chip_id     # prints the concrete chip
```

**Tooling install:**
```bash
pip install esptool                 # chip_id / flash / read_mem
cargo install espflash              # alt flasher
brew install open-ocd               # verify it ships target/esp32c3.cfg + esp32s3.cfg;
                                    # else use github.com/espressif/openocd-esp32 releases
```
