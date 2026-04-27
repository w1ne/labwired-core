# Case Study: ESP32-S3 Plan 3 — GPIO + IO_MUX + Interrupt Matrix + SYSTIMER alarms

**Date closed:** 2026-04-27
**Branch:** `feature/esp32s3-plan3-gpio-blinky`
**Spec:** `docs/superpowers/specs/2026-04-26-plan-3-gpio-intmatrix-blinky.md`
**Predecessor:** Plan 2 (`docs/case_study_esp32s3_plan2.md`)
**Milestone closed:** M4 (Blinky + GPIO interrupt) from the ESP32-S3-Zero design spec.

---

## What Plan 3 Delivered

A real `esp-hal` Rust binary that toggles GPIO2 from a SYSTIMER alarm ISR every 500 ms now runs end-to-end in the LabWired simulator. The same firmware that toggles a logic-analyzer-probable pin on a connected ESP32-S3-Zero also produces visible GPIO transitions in the simulator's tracing log and structured `--gpio-trace` JSONL stream:

```
$ labwired-cli run --chip configs/chips/esp32s3-zero.yaml \
                   --firmware …/esp32s3-blinky
labwired-cli run: entry=0x40378e20 stack=0x3fcdb700 segments=5
INFO gpio: GPIO2: 0->1  (cycle=40008445)
INFO gpio: GPIO2: 1->0  (cycle=80008446)
INFO gpio: GPIO2: 0->1  (cycle=120008445)
…  12 transitions in 500 M sim cycles, exactly 40 M apart  …
```

40 M cycles at 80 MHz CPU = 500 ms — bit-identical to the firmware's `Duration::from_millis(500)`.

### Test counts (final state)

| Suite | Passing | Notes |
|---|---|---|
| `labwired-core` (unit + integration, sim suite) | 615 | +47 from Plan 2 close, mostly new GPIO + IO_MUX + intmatrix + alarm tests |
| `labwired-core --features esp32s3-fixtures` | +2 | `e2e_hello_world` (Plan 2) + `e2e_blinky` (Plan 3) |
| Total (sim suite, excluding cross-compile crates) | 615 | `cargo test --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture --exclude arm-hello --exclude riscv-hello --exclude demo_blinky -- --skip test_demo_blinky_gpio_toggle` |

### Components shipped

| Component | File | LoC (approx) |
|---|---|---|
| GPIO peripheral + GpioObserver trait | `crates/core/src/peripherals/esp32s3/gpio.rs` | ~375 |
| IO_MUX peripheral | `crates/core/src/peripherals/esp32s3/io_mux.rs` | ~160 |
| Interrupt Matrix peripheral + Bus IRQ aggregation | `crates/core/src/peripherals/esp32s3/intmatrix.rs`, `bus/mod.rs`, `lib.rs` | ~425 |
| SYSTIMER alarms + TRM offset rework | `crates/core/src/peripherals/esp32s3/systimer.rs` | ~645 (significant rework) |
| CPU pending_irq aggregation hook | `crates/core/src/cpu/xtensa_lx7.rs`, `bus/mod.rs` | ~85 |
| Decoder fixes (S32E group, SRLI shamt) + new instructions | `crates/core/src/decoder/xtensa.rs` | ~120 |
| System glue (clear catch-all order, register GPIO/IO_MUX/intmatrix) | `crates/core/src/system/xtensa.rs` | ~100 |
| TracingGpioObserver + JsonGpioObserver + `--gpio-trace` flag | `crates/cli/src/gpio_observer.rs`, `main.rs` | ~110 |
| Example firmware (esp-hal blinky on GPIO2) | `examples/esp32s3-blinky/` | ~140 |
| Integration test (hand-rolled ISR full IRQ chain) | `crates/core/tests/intmatrix_alarm.rs` | ~150 |
| E2E test (real firmware in sim, recording observer) | `crates/core/tests/e2e_blinky.rs` | ~125 |
| Total | | ≈2,500 (`+2,484 / −192` per `git diff --stat` Plan 2…Plan 3) |

---

## Plan Corrections Caught During Implementation

These are the simulator bugs Plan 3's iteration loop surfaced — issues the plan's author didn't anticipate but real firmware (and the assembler) revealed.

| # | Issue | Resolution |
|---|---|---|
| 1 | **SYSTIMER offsets drifted from TRM.** Plan 2 had LOAD_HI/LO at 0x18-0x24 and LOAD-commit at 0x60/0x64; TRM has them at 0x0C-0x18 and 0x5C/0x60. Hello-world worked accidentally because Delay only used 0x04 + 0x40/0x44 (already correct). esp-hal's Alarm API would have written to TRM-correct offsets that landed in our LOAD_HI/LO fields. | Reworked SYSTIMER to TRM-correct offsets across the board. All Plan 2 tests updated; hello-world still passes. |
| 2 | **TARGETx_CONF bit semantics flipped.** Plan 3 Task 4 used bit 30 for enable; TRM (verified via `esp-pacs` `target_conf::W`) has bit 31 for enable, bit 30 for auto-reload. | Bit semantics corrected; all alarm tests updated. |
| 3 | **First-match-wins peripheral resolution + catch-all stub shadowing.** Plan 3 added GPIO at 0x6000_4000, IO_MUX at 0x6000_9000, intmatrix at 0x600C_2000 — all inside Plan 2 catch-all stubs (`low_mmio`, `rtc_cntl`, `system`). The implementer documented "must register before catch-alls" but registered AFTER. `SystemBus` iterates first-match-wins, so the catch-alls shadowed the real peripherals. | Specific peripherals moved to register before catch-alls in `configure_xtensa_esp32s3`. Integration test (`intmatrix_alarm`) caught this. |
| 4 | **S32E/L32E decoder under wrong opcode group.** Plan 1 placed S32E/L32E at op0=9 (with a confusing CD-narrow-collision workaround in the step function). Real `xtensa-esp32s3-elf-as` emits S32E with bytes that decode as op0=0, op1=9 (QRST group) — the Plan 1 placement was a guess that happened to match buggy oracle test bytes. | Decoder reworked to put S32E/L32E in QRST op1=9. Plan 1 oracle test bytes regenerated with the assembler. |
| 5 | **SRLI shamt field came from `t` instead of `s`.** All existing SRLI tests happened to use `at == s` so no test caught the swap. esp-hal's `(prid >> 13) & 1` CPU-discrimination check decoded with the wrong shift amount, sending the ISR down a path that read from cpu1's INTR_STATUS register (which we don't model), getting stale data. | Decoder fixed; regression test added. |
| 6 | **intmatrix didn't model PRO_INTR_STATUS_REG_0..3.** esp-hal's `__level_1_interrupt` reads these four 32-bit status registers (offsets 0x18C..0x19C in the intmatrix peripheral) to discover which source asserted. Without them, the snapshot was always zero so the iterator yielded nothing and user ISRs never ran. | Bus aggregator now updates these from peripheral `explicit_irqs` each tick. Three new unit tests. |
| 7 | **SYSTIMER source ID was 79 in our model but 57 per the actual TRM intmatrix table.** | Updated the SYSTIMER peripheral to emit 57; integration test updated. |

---

## ROM Thunks Added

Plan 3 didn't require new ROM thunks — esp-hal blinky calls the same set hello-world did.

---

## Plan 3 Exit Criteria Status

| # | Criterion | Status |
|---|---|---|
| 1 | Sim suite stays green | PASS — 615 tests passing |
| 2 | esp-hal blinky builds | PASS — 2.1 MB ELF |
| 3 | Integration test passes (hand-rolled ISR full IRQ chain) | PASS — `intmatrix_alarm_full_irq_chain` |
| 4 | E2E demo ticks LED | PASS — `e2e_blinky` test verifies 4+ GPIO2 transitions in ≤480 M sim cycles |
| 5 | CLI runs the firmware end-to-end | PASS — `labwired-cli run` shows alternating `GPIO2: 0->1` / `GPIO2: 1->0` lines at 40 M-cycle intervals |
| 6 | `--gpio-trace path.json` produces valid JSONL | PASS — verified manually |
| 7 | Documentation | PASS — this case study |

---

## Known Gaps and Acknowledged Limitations

- **No WS2812 / RMT.** The iconic S3-Zero RGB LED demo is Plan 3.5 territory; Plan 3 hits the alarm-driven IRQ path with a logic-analyzer-probable plain GPIO toggle.
- **No HW oracle GPIO diff.** The `--diff` stretch goal would compare sim and HW GPIO transitions over a fixed window — deferred (OpenOCD GPIO polling has unbounded latency without a logic analyzer).
- **GPIO0..31 only.** GPIO32..48 (the high half of `OUT1`/`IN1`/`STATUS1`) is not yet wired into the peripheral.
- **GPIO input interrupts not routed.** Edge-detect registers exist in the GPIO peripheral but the GPIO source ID isn't yet bound to the intmatrix.
- **UNIT1 alarms not modelled.** Only UNIT0 alarms emit IRQs; UNIT1 alarm registers round-trip but never fire.
- **TIMG / WDT.** Separate timer block, not modelled.
- **Multi-core (cpu1).** Still single-core. APP-side intmatrix mapping registers silently accept writes; PRO-side is the only consumer.

---

## Invitation for Plan 4

Plan 3 closes the M4 milestone. Plan 4 builds on this for the M5 milestone — sensor I/O:

- **I²C0:** START/STOP, ACK/NACK, SCL/SDA waveform.
- **SPI2 (GP-SPI):** transaction sequencer, MOSI/MISO/SCK/CS, 4 CS lines.
- **External device: tmp102** I²C temperature sensor as a behavioral peripheral.
- **First sensor-read demo:** esp-hal firmware that reads tmp102 over I²C and prints the temperature via USB_SERIAL_JTAG.

The peripheral oracle pattern from Plan 1 + the GpioObserver pattern from Plan 3 generalize to bus-protocol observers for I²C/SPI traces.
