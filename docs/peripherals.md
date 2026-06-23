# Peripheral modeling guide

How to add a peripheral to LabWired and validate it against real silicon. This is
the single canonical reference; it covers both modeling paths, the `Peripheral`
trait contract, the validation loop, and the merge bar.

> **Golden rule:** a peripheral is *onboarded* only when its modeled register
> behavior has been diffed against real silicon and any divergence is understood.
> Code that compiles and passes a hand-written unit test is *modeled*, not
> *validated* — and that difference is the whole point of LabWired.

## 1. Pick a path

| Path | When | Where |
|------|------|-------|
| **Declarative YAML** | Register-file blocks: reset values, RAZ/WI, W1C/W0C, clear-on-read, simple delayed flag/IRQ effects. | `configs/peripherals/<chip>/<periph>.yaml` + a `type: declarative` entry in `configs/chips/<chip>.yaml`. Generate the boilerplate with `svd-ingestor`. |
| **Rust model** | Behavioral blocks: timers, UART/I²C/SPI engines, DMA, PWM, crypto — anything with non-trivial `tick()` logic, FIFOs, or cross-peripheral side-effects. | `crates/core/src/peripherals/<periph>.rs` (shared) or `…/<chip>/<periph>.rs` (chip-specific). |

Reach for declarative first; drop to Rust only when the behavior outgrows what the
descriptor can express (§2).

## 2. Declarative path

`GenericPeripheral` (`crates/core/src/peripherals/declarative.rs`) serves a
`PeripheralDescriptor` directly — no Rust per peripheral. From the descriptor it
handles, automatically:

- register layout, sizes, and **reset values**;
- access permissions (R/W/RO) with bounds checking — violations raise a BusFault;
- byte-granular reconstruction of 16/32-bit registers;
- **side-effects**: `read_action: clear`, `write_action: oneToClear | zeroToClear`;
- **timing**: periodic and delayed actions (`SetBits` / `ClearBits` /
  `WriteValue` on a named register) that can also raise an interrupt.

Clock gating is expressed at the chip level, not in the descriptor: a `clock:`
field on the peripheral entry in `configs/chips/<chip>.yaml` binds it to an RCC
enable bit, so an unclocked peripheral reads 0 / drops writes exactly like silicon.

The full register schema is documented in
[`declarative_registers.md`](declarative_registers.md). Generate a starting point:

```bash
cargo run -p svd-ingestor -- --input STM32F4.svd --filter USART1 \
  --output-dir configs/peripherals/<chip>
```

Then hand-tune reset values and field semantics against the reference manual. If a
behavior can't be expressed declaratively, model it in Rust (§3) — don't bend the
descriptor into something it isn't.

## 3. Rust path

Implement the `Peripheral` trait:

```rust
pub trait Peripheral: std::fmt::Debug + Send {
    fn read(&self, offset: u64) -> SimResult<u8>;            // CPU read
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()>; // CPU write
    fn tick(&mut self) -> PeripheralTickResult;             // one step of time
}
```

- **Byte granularity.** The bus does every transaction one byte at a time; a 32-bit
  `STR` arrives as four writes. Align with `offset & !3` to find the register, then
  shift the byte. Define unmapped behavior (RAZ/WI) explicitly.
- **Side-effects in `write`** (and a separate side-effect-free read for the
  debugger): W1C flag clears, FIFO pops, start bits.
- **Deterministic `tick()`.** No wall-clock, no `thread::sleep`, no RNG — behavior
  depends only on the tick count. It's on the hot path, so decimate high-frequency
  logic with a cycle counter. Emit IRQs / DMA / cross-peripheral writes through
  `PeripheralTickResult` (`irq`, `explicit_irqs`, `dma_requests`, `dma_signals`,
  `mmio_writes`, `cycles`) rather than touching globals.

### Behavioral patterns

- **DMA** uses a two-phase request/execute model: a peripheral returns
  `DmaRequest`s from `tick()`, and the `SystemBus` arbitrates and performs the
  memory operations afterward (the bus is the master; peripherals never hold it).
- **EXTI/AFIO** map GPIO pins to interrupt lines; AFIO registers select which port
  drives each line. See [`hardware_interaction_guide.md`](hardware_interaction_guide.md)
  for the DMA/IRQ propagation detail through the two-phase tick.

## 4. Validate against silicon (capture → model → oracle → validate)

The L476 path (`crates/core/tests/firmware_survival.rs`) is the canonical
"done right" example. Scaffolding lives in `scripts/hw-oracle/` and `crates/hw-oracle`.

1. **Capture.** Flash a probe firmware that exercises *only* the target peripheral,
   then record a baseline under `scripts/hw-oracle/captures/<chip>/<ts>/`: PC trace
   plus pre/post MMIO+RAM snapshots of the checkpoint windows. Add the peripheral's
   MMIO window (and any RAM it touches) to the checkpoint regions.
2. **Model** per §2/§3.
3. **Oracle diff.** Replay the same ELF in-sim and diff against the capture:
   ```bash
   cargo run --release -p labwired-hw-oracle --bin <chip>_replay_in_sim -- \
     --capture scripts/hw-oracle/captures/<chip>/<ts> --elf <same-firmware.elf>
   ```
   It reports the first divergence — a wrong reset value, missing field, mis-timed
   flag, or absent side-effect. Fix and repeat until clean (or document the residual).
4. **Lock it in.** Add a survival/parity test asserting byte-for-byte parity on the
   validated behavior. That's what the CI gates protect.

Bus-wire snooping (SPI/I²C line traffic) is out of scope — validate those at the
register/FIFO level. Radio/BLE blocks are firmware-blob-driven: either model the
controller registers against captures, or thunk the API/ROM call and route the data
plane through the in-sim `SimNet`; record which strategy per chip in its board doc.

## 5. Standards

- **Determinism** is non-negotiable: identical inputs + instruction count ⇒
  identical state, regardless of host speed.
- **No `unsafe`** in peripheral code (barring explicitly approved low-level bridges).
- **Serializable state** — snapshots and fuzzing depend on it; avoid raw pointers
  and unstable-iteration containers.
- **AI-generated models are held to the same bar.** A model that "looks right" but
  fails the oracle diff or the timing/side-effect assertions does not merge; an
  autonomous agent must close the loop (compile → run → diff) before proposing it.
- Fixing behavior to match the datasheet is a **patch**, even though it changes
  simulation output — the previous behavior was wrong by definition.

## 6. Definition of done

1. Builds for the target(s); `cargo fmt` / `clippy` clean.
2. Survival/parity test added and green (§4.4).
3. Oracle diff against a committed capture clean, or residual documented in the
   board's `VALIDATION.md`.
4. Relevant CI green: `core-ci.yml`, `core-onboarding-smoke.yml`, the per-arch
   `core-board-ci-fixture-*.yml`, `core-unsupported-audit.yml`,
   `core-validate-hw-targets.yml`.
5. `examples/<board>/` updated (README, `VALIDATION.md`, the probe firmware/script).

## Appendix — capture front-ends per architecture

The loop is identical; only how you halt the core, read PC, and dump memory differs.

| Arch | Examples | Probe / transport | Notes |
|------|----------|-------------------|-------|
| **Cortex-M** | STM32F1/F4/H5, RP2040, nRF52840 | ST-Link / CMSIS-DAP via OpenOCD SWD | `scripts/hw-capture-stm32*.sh`, `hw-capture-nrf52840.sh`; mind connect-under-reset. |
| **RISC-V** | ESP32-C3 | built-in USB-Serial/JTAG via `board/esp32c3-builtin.cfg` | distro OpenOCD lacks esp32c3 targets — use Espressif's `openocd-esp32` fork. |
| **Xtensa** | ESP32-S3, ESP32-WROOM | built-in USB-JTAG (S3) / ESP-Prog (WROOM) | `scripts/hw-oracle/esp32_capture.sh`; cross-check the decoder vs `xtensa-esp-elf-objdump`. |

For whole-*board* bring-up (chip YAML + system manifest + smoke firmware), see
[`board_onboarding_playbook.md`](board_onboarding_playbook.md).
