# BBC micro:bit v2 (Nordic nRF52833) — L3 (production-ready)

The **micro:bit v2** target MCU: Nordic **nRF52833** — Cortex-M4F at 64 MHz,
512 KB flash, 128 KB RAM, 42 GPIOs, 2.4 GHz radio, USB and NFC. LabWired
models the nRF52 peripheral family shared with the nRF52840; this page covers
the **smoke-manual** slice (core, GPIO P0/P1, CLOCK, and the UARTE0 EasyDMA
console bridged to the interface MCU) plus the **tier-1 peripheral-depth**
fixture, which raw-register-exercises all twelve tier-1 classes on this silicon
row — including EasyDMA (`dma`) and real NVIC interrupt delivery (`irq`).

The honest boundary of every claim on this page is
[`examples/microbit-v2/KNOWN_LIMITATIONS.md`](../../examples/microbit-v2/KNOWN_LIMITATIONS.md);
read it before treating any block below as supported.

!!! tip "Live status"
    - [Chip conformance](../coverage/chip-conformance.md)
    - [Bus visibility](../coverage/bus-visibility.md)
    - [Target support rubric](../target_support_rubric.md)

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/nrf52833.yaml`](../../configs/chips/nrf52833.yaml) |
| System | [`configs/systems/microbit-v2.yaml`](../../configs/systems/microbit-v2.yaml) |
| Example | [`examples/microbit-v2/`](../../examples/microbit-v2/README.md) |
| Committed ELF | `tests/fixtures/microbit-v2-smoke.elf` |
| Survival gate | `firmware_survival::test_nrf52833_microbit_v2_smoke_survival` |
| Known limitations | [`examples/microbit-v2/KNOWN_LIMITATIONS.md`](../../examples/microbit-v2/KNOWN_LIMITATIONS.md) |
| Selected motion model | [LSM303AGR](../engineering/microbit-lsm303agr.md): declarative accelerometer + magnetometer, proven by a source-built guest |
| Validation runbook | [`examples/microbit-v2/VALIDATION.md`](../../examples/microbit-v2/VALIDATION.md) |
| Tier-1 fixture | [`examples/tier1-fixture/nrf52833/`](../../examples/tier1-fixture/nrf52833/) → `tests/fixtures/tier1/nrf52833.elf` |
| Tier-1 result | **all 12 classes PASS** — clock/gpio/uart/timer/dma/irq (the six rubric classes) plus i2c/spi/adc/wdt/pwm/rtc |
| Instruction audit | 0 unknown Thumb-16 / 0 unhandled Thumb-32 over 200 000 steps (100 % coverage) |
| Tier | **L3 (production-ready)** — L2 (CI cell + limitations + audit) with all six rubric classes `pass`; no silicon bench diff |

---

## Flash / firmware artifact

| Use | Artifact | Notes |
|-----|----------|-------|
| Bare-metal / Zephyr | **ELF** | Vector table at flash base `0x00000000`; 512 KB / 128 KB per nRF52833 PS |
| micro:bit runtime (codal) | — | not exercised in this repo |
| UF2 / SoftDevice | — | not modelled |

---

## Pins (micro:bit v2)

Nordic pins are **P0.xx / P1.xx**. Values below are from the micro:bit v2
hardware docs (`tech.microbit.org/hardware/schematic/`); the schematic's
`UART_INT_*` labels are from the interface MCU's perspective.

| Signal | nRF52833 pin | Notes |
|--------|--------------|-------|
| UART TX (target → interface MCU) | **P0.06** | `MICROBIT_PIN_UART_TX` in codal-microbit-v2 |
| UART RX (interface MCU → target) | **P1.08** | labels are swapped on the schematic; see microbit-foundation/microbit-v2-hardware#5 |
| Button A | **P0.14** | active-low; declared as an `input` stub in `board_io` |
| Button B | **P0.23** | active-low; declared as an `input` stub in `board_io` |
| 5×5 LED matrix rows/cols | P0.19/P0.21/P0.22/P0.15/P0.24 + P0.28/P0.11/P0.31/P1.05/P0.30 | Row/column multiplexed; GPIO/GPIOTE pad levels drive the matrix |
| Selected LSM303AGR motion sensor | Internal I²C / `i2c0` | Accelerometer 7-bit address `0x19`, magnetometer `0x1e`; alternative FXOS8700 variant not selected |
| Shared sensor interrupt | P0.25 | Open-drain board connection; not wired or qualified in the bounded motion model |

P1 is P1.00–P1.09 on this part (the family total is 42 GPIOs). Firmware uses
the silicon P1 base `0x50000300`: OUT is `0x50000804`, DIR is `0x50000814`
and PIN_CNF starts at `0x50000A00`. The compact descriptor window begins at
`0x50000800` with register offset `0x500`, so it no longer steals P0 PIN_CNF
or requires firmware to use the old simulator-only `0x50001000` remap.

---

## Support matrix

| Block | Status | Notes |
|-------|--------|-------|
| Cortex-M4F + FPU | ✅ | Thumb-2 decoder shared with nRF52840; clean instruction audit |
| NVIC / interrupt delivery | ✅ tier-1-proven | TIMER0 COMPARE0 → NVIC IRQ 8; the vector handler runs (real exception path) |
| EasyDMA (`dma`) | ✅ tier-1-proven | descriptor proof on SAADC RESULT (`PTR`/`MAXCNT`/`AMOUNT` + payload); class declared by YAML opt-in — nRF52 has no central DMA controller |
| GPIO P0/P1 | ✅ tier-1-proven | `profile: nrf52`; P0.13 + P1.05 round-trips; buttons declared as inputs |
| UARTE0 EasyDMA console | ✅ tier-1-proven | `OK\n` smoke on P0.06 at 115200, `ENABLE=8`, ENDTX poll; the TIER1 transcript is the tier-1 proof |
| CLOCK | ✅ tier-1-proven | `TASKS_HFCLKSTART` → `EVENTS_HFCLKSTARTED` + `HFCLKRUN` |
| TIMER0 / RTC0 / PWM0 / SPIM2 / TWIM1 / SAADC / WDT | ✅ tier-1-proven | first instance of each class; see the transcript below |
| NVMC / FICR / UICR / ECB / AAR / RNG / TEMP / EGU / QDEC / COMP / PDM / I2S / PPI / GPIOTE | ⚠️ declared | register/behavioural models shared with nRF52840; not exercised on this part |
| RADIO / BLE | ⚠️ digital layers only | registers + EasyDMA + whitening/CRC/address matching modelled; idealized lossless air, **no BLE stack/link layer**, not exercised on this board |
| USB (USBD) | ⚠️ window only | register surface; no enumeration or endpoint state machine |
| NFC (NFCT) | ⚠️ window only | register surface; no tag/carrier or peer |
| 5×5 LED matrix | ✅ functional pad model | `led-matrix-mux`, GPIO/GPIOTE/PPI paths and integrated grayscale display; no electrical current/light sensing |
| LSM303AGR accelerometer / magnetometer | ✅ declarative, guest-proven | Separate `i2c0` components with live x/y/z inputs; data-ready paced at 100 Hz (ODR value and BDU not decoded); shared IRQ/FIFO/gestures unsupported |
| Analog microphone / SAADC | ⚠️ held input only | P0.05/AIN3 levels may be injected through the bounded SAADC API; no continuous microphone capture |
| Speaker / touch logo | ❌ not attached | Audio playback and capacitive sensing remain qualification gaps |
| Silicon diff / executing-fidelity differential | ❌ none | no bench part captured; every claim is simulator-derived |

---

## What is proven

`crates/firmware-nrf52833-demo` is a bare-metal `no_std` image: it programs
`PSEL.TXD = P0.06`, `PSEL.RXD = P1.08`, `BAUDRATE = 0x01D6_0000` (115200),
`ENABLE = 8` (UARTE), then pushes a RAM-resident `OK\n` through
`TXD.PTR`/`TXD.MAXCNT`/`TASKS_STARTTX` and waits for `EVENTS_ENDTX`. The
committed ELF is booted in-process by the survival gate and the byte stream is
asserted at the UART sink; the same binary runs on silicon-target toolchains
(`thumbv7em-none-eabi`).

The chip descriptor and system manifest are also gated by
`nrf52833_from_config_builds` (`SystemBus::from_config` must expose `uart0`,
`gpio0`, `gpio1`).

At L3 the six rubric classes are proven by the tier-1 fixture: **clock, gpio,
uart, timer, dma, irq**. `dma` is EasyDMA (the nRF52 has no central DMA
controller), declared per-chip by `tier1_classes: ["dma"]` on the EasyDMA
blocks in the chip YAML; `irq` is a peripheral-sourced TIMER0 COMPARE0
interrupt serviced through the NVIC. The full 12-class transcript, the exact
run command and the clean instruction audit are in
[`examples/microbit-v2/VALIDATION.md`](../../examples/microbit-v2/VALIDATION.md).

## What is not proven

No silicon capture, no register sweep, no executing-fidelity differential. The
BLE stack and radio medium, USB protocol, NFC tag interaction, the
complete on-board sensor protocols and speaker/microphone paths remain unqualified (see
[known limitations](../../examples/microbit-v2/KNOWN_LIMITATIONS.md)). The chip
descriptor mirrors the nRF52840 family's peripheral types — the shared blocks
are the same silicon IP; the tier-1 fixture exercises all twelve classes
end-to-end, but only the first instance of each class, and per-instance
sweeps, cycle-accurate timing and the radio/analog paths stay unproven.

---

## Tier-1 peripheral depth

[`examples/tier1-fixture/nrf52833/`](../../examples/tier1-fixture/nrf52833/) is
a standalone bare-metal `no_std` image (own `[workspace]`, thumbv7em) that
pokes raw MMIO and prints the TIER1 protocol over the UARTE0 EasyDMA console
configured exactly as the micro:bit v2 wiring (`PSEL.TXD = P0.06`,
`PSEL.RXD = P1.08`, 115200, `ENABLE = 8`):

```text
TIER1 gpio PASS      # P0.13 and P1.05 DIRSET/OUTSET/OUT/OUTCLR round-trips
TIER1 clock PASS     # TASKS_HFCLKSTART -> EVENTS_HFCLKSTARTED + HFCLKRUN
TIER1 timer PASS     # TIMER0 32-bit counter advances between two CAPTURE0s
TIER1 irq PASS       # TIMER0 COMPARE0 -> NVIC IRQ 8, DefaultHandler runs
TIER1 rtc PASS       # RTC0 COUNTER advances from TASKS_START
TIER1 i2c PASS       # TWIM1 EasyDMA TX with no slave -> ANACK + LASTTX/ERROR
TIER1 spi PASS       # SPIM2 EasyDMA TXD/RXD round-trip -> END + AMOUNTs
TIER1 adc PASS       # SAADC EasyDMA conversion read back by value (12/10 bit)
TIER1 dma PASS       # SAADC RESULT EasyDMA: 2 pointers, MAXCNT 4/2, sentinels
TIER1 wdt PASS       # WDT CRV/RREN + TASKS_START -> RUNSTATUS + TIMEOUT
TIER1 pwm PASS       # PWM0 SEQ[0] EasyDMA playback -> SEQEND0 + PWMPERIODEND
TIER1 done           # UART is implicit: the transcript itself is the proof
```

Run it (the step budget is generous: the whole sequence completes inside
200 000 steps):

```bash
labwired run --chip configs/chips/nrf52833.yaml \
  --firmware tests/fixtures/tier1/nrf52833.elf --max-steps 8000000
```

**Honest limitations.** Only the first instance of each class is exercised
(TIMER0, RTC0, PWM0, TWIM1, SPIM2 — SPIM3/PWM1-3/RTC1-2 are declared but
unswept). The `i2c` proof is a no-slave address-NACK, not a data transfer
against the separately selected LSM303AGR components; SAADC in this historical
fixture reads the compatibility source rather than a held injected input.
The regenerated GPIO P1 fixture uses silicon addresses rather
than the historical simulator remap. The `dma` class is EasyDMA (there is
no central DMA controller on this silicon) and is declared by an explicit
per-chip YAML opt-in; the check proves descriptor semantics on the SAADC
RESULT channel, not a sweep of every EasyDMA engine. There is still no silicon
diff for any of these paths — the full list is in
[known limitations](../../examples/microbit-v2/KNOWN_LIMITATIONS.md).

### Instruction audit

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/tier1/nrf52833.elf \
  --system configs/systems/microbit-v2.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/microbit-v2
```

Observed: `unknown_thumb16: 0`, `unhandled_thumb32: 0`, `unknown_riscv: 0`,
`unsupported_total: 0`, 199 999 instructions executed at 100 % support; the
full TIER1 transcript is already emitted by that step. Artifacts:
`out/unsupported-audit/microbit-v2/report.md` and `metrics.json` (reproduce
with [`examples/microbit-v2/VALIDATION.md`](../../examples/microbit-v2/VALIDATION.md)).

### CI lanes

- **L1 smoke cell** — `microbit-v2` in
  [`.github/workflows/core-coverage-matrix-smoke.yml`](../../.github/workflows/core-coverage-matrix-smoke.yml)
  rebuilds `firmware-nrf52833-demo` and runs `examples/microbit-v2/uart-smoke.yaml`.
- **Tier-1 matrix + ratchet** — `cargo test --release -p labwired-cli --test
  tier1_matrix --test tier1_matrix_ratchet` in the nightly `core-ci.yml` full
  job exercises every committed fixture, including this one.
- **Fixture drift** — the `tier1-fixture-drift` job in
  [`.github/workflows/core-nightly.yml`](../../.github/workflows/core-nightly.yml)
  rebuilds all tier-1 blobs weekly and fails on any sha256 drift from
  `tests/fixtures/tier1/MANIFEST.json`.
- No per-chip workflow file is needed; the chip is picked up by the shared
  lanes above.

---

## How to run

```bash
cargo build -p firmware-nrf52833-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- \
  --firmware tests/fixtures/microbit-v2-smoke.elf \
  --system configs/systems/microbit-v2.yaml \
  --max-steps 200000
```

Expected: the run finishes and prints `OK` to stdout; the deterministic
scripted assertion lives in
[`examples/microbit-v2/uart-smoke.yaml`](../../examples/microbit-v2/uart-smoke.yaml).

---

## Related

- [nRF52840](nrf52840.md) — the silicon-verified sibling this descriptor borrows its blocks from
- [nRF52832](nrf52832.md) — single-port sibling
- [micro:bit v2 example](../../examples/microbit-v2/README.md)
- [Known limitations](../../examples/microbit-v2/KNOWN_LIMITATIONS.md)
- [Validation runbook](../../examples/microbit-v2/VALIDATION.md)
