# micro:bit v2 (nRF52833) — Known limitations

This file is the honest boundary of what LabWired claims for the **BBC
micro:bit v2** target (Nordic **nRF52833**, Cortex-M4F, 64 MHz). It is the
known-limitations artifact required by the
[target support rubric](../../docs/target_support_rubric.md) for L2/L3, and it
is scoped to the **documented scenarios** below — not to every firmware the
part could run.

Everything here is **simulator-derived**. No physical micro:bit v2 was
bench-diffed against this simulator (no reset-state capture, no register sweep,
no executing-fidelity differential on this part), so every claim below is a
model claim, not a silicon measurement.

---

## Proven at L3 (documented scenarios)

The six rubric classes are all proven by the committed Tier-1 fixture
[`examples/tier1-fixture/nrf52833/`](../../examples/tier1-fixture/nrf52833/)
(`tests/fixtures/tier1/nrf52833.elf`, sha256 pinned in
`tests/fixtures/tier1/MANIFEST.json`), which pokes raw MMIO and reports one
`TIER1 <class> PASS` line per class over the UARTE0 EasyDMA console wired
exactly as the board (`PSEL.TXD = P0.06`, `PSEL.RXD = P1.08`, 115200,
`ENABLE = 8`):

| Class | Scenario proven |
|-------|-----------------|
| clock | CLOCK `TASKS_HFCLKSTART` → `EVENTS_HFCLKSTARTED` + `HFCLKRUN` |
| gpio | P0.13 and P1.05 `DIRSET`/`OUTSET`/`OUT`/`OUTCLR` round-trips on both ports |
| uart | the whole TIER1 transcript itself: EasyDMA TX (`TXD.PTR`/`MAXCNT`/`TASKS_STARTTX`, `EVENTS_ENDTX`-polled) on the board wiring |
| timer | TIMER0 32-bit free-run; two `TASKS_CAPTURE0` samples prove progress |
| dma | **EasyDMA** descriptor semantics on the SAADC RESULT channel: two destination pointers, `MAXCNT` 4 then 2, sentinels proving the write length, and the exact converted payload in the pointed-to words. The class is declared by the chip YAML opt-in `tier1_classes: ["dma"]` (nRF52 has no central DMA controller) |
| irq | real peripheral-sourced NVIC delivery: TIMER0 COMPARE0 + `INTENSET` pends NVIC IRQ 8, and a `DefaultHandler` counts the vector actually running |

Additional tier-1 classes in the same fixture: `i2c` (TWIM1 EasyDMA TX to an
absent address → modeled `ANACK` + `EVENTS_ERROR`/`LASTTX`), `spi` (SPIM2
EasyDMA TXD/RXD → `EVENTS_END` + both `AMOUNT`s), `adc` (SAADC conversion read
back **by value** at 12 and 10 bits), `wdt` (CRV/RREN → `RUNSTATUS` +
`TIMEOUT`), `pwm` (PWM0 `SEQ[0]` EasyDMA playback → `SEQEND0` +
`PWMPERIODEND`), `rtc` (RTC0 `COUNTER` advances from `TASKS_START`).

The L1 smoke path (reset, vector table, UARTE0 console banner `OK`) is gated in
CI (`core-coverage-matrix-smoke.yml`, microbit-v2 cell) and by the in-process
survival gate `firmware_survival::test_nrf52833_microbit_v2_smoke_survival`.

---

## Partially modelled

- **RADIO / BLE** — the digital layers of the nRF52 RADIO block are modelled
  (register surface, EasyDMA `PACKETPTR`, PN9 whitening, CRC-24 with a real
  verdict, address matching, and cross-instance delivery through an in-process
  registry). The **RF medium is idealized**: lossless, collision-free, no bit
  errors, no interference. There is **no BLE stack, no link layer and no
  SoftDevice**, and no committed fixture exercises RADIO on the micro:bit v2
  row — treat BLE firmware as unsupported.
- **USB (USBD)** — register surface only. Host enumeration, SETUP packet
  handling and the endpoint state machine are not modelled; a firmware that
  waits for a real bus event will not get one. micro:bit v2 USB console /
  DAPLink paths are unsupported.
- **NFC (NFCT)** — register surface only (`FRAMEDELAY`, `NFCID`,
  `PACKETPTR`, task/event round-trips). No carrier, tag protocol or peer.
- **I²C / SPI transfers** — the tier-1 proofs are a no-slave TWIM
  address-NACK and a SPIM2 transfer with no MISO driver attached (the model's
  floating line reads 0). The production system now selects separate
  LSM303AGR accelerometer/magnetometer components on `i2c0`; this does not
  upgrade the old tier-1 NACK scenario to a sensor proof. See the separate
  [motion-model contract](../../docs/engineering/microbit-lsm303agr.md).
- **SAADC input** — held millivolt levels may be injected on AIN0..7;
  the microphone pin is P0.05/AIN3. The old fixture retains its internal source
  before any input injection. This is a bounded digital conversion model, not
  a microphone waveform, electrical analog front end or continuous capture.
  See the [held-input contract](../../docs/engineering/nrf52-saadc-held-inputs.md).
- **GPIO P1 window** — the nRF52833 descriptor now uses silicon register
  addresses with a compact non-overlapping window; historical simulator-remap
  firmware must be rebuilt. The separate nRF52840 descriptor is unchanged.
- **GPIO reset fidelity** — the shared model initializes PIN_CNF to0 rather
  than the documented disconnected-input value2. The executable board guest
  explicitly configures its display/button pins; it is not a reset-value sweep.
- **WDT** — the timeout signal is observed; core reset on bite is deliberately
  not triggered in the fixture (the model surfaces the event without resetting).
- **PWM** — sequence playback and events are proven; the driven pad waveform
  is not probed at a pin.
- **nRF52 TIMER timing** — ordering of events is preserved, but the model
  advances one base tick per CPU step rather than at 16 MHz wall-clock; no
  cycle-budget calibration has been done for this family.
- **Peripheral instance coverage** — only the **first instance per class** is
  swept: TIMER0 (not TIMER1–4), RTC0 (not RTC1–2), PWM0 (not PWM1–3), SPIM2
  (not SPIM3 or the SPIM0/TWIM0 shared window), TWI1 (not TWIM0), SAADC CH0
  only. The other instances share the same models but are not exercised.
- **PPI / GPIOTE / EGU** — register models exist and are declared, but no
  tier-1 check drives a PPI-chained task or a GPIOTE interrupt.

---

## Remaining model limitations

- **BLE / Bluetooth stack** and any radio medium realism (see above).
- **USB device protocol** (enumeration, classes, CDC) — register window only.
- **NFC tag/carrier interaction** — register window only.
- **5×5 LED matrix** — row/column multiplexed and modeled by integrated
  GPIO/GPIOTE pad duty. Ambient-light sensing, LED current/voltage and analog
  brightness calibration are not modeled.
- **Motion sensors** — the selected LSM303AGR variant exposes separate
  `accelerometer` (`0x19`) and `magnetometer` (`0x1e`) I²C components.
  Live `x`/`y`/`z` inputs are held acceleration in g / magnetic field in µT,
  reported live; all inputs default to zero, with no invented gravity or
  motion. Data-ready paces at a fixed 100 Hz while each half is measuring;
  the configured ODR value and BDU latching are not modelled. This is not the alternative FXOS8700-equipped board. Shared
  open-drain P0.25 sensor IRQ, FIFO, gestures,
  self-test, temperature and physical calibration are unsupported. Full CODAL /
  MakeCode sensor firmware and motion/audio browser-WASM qualification remain
  pending; this does not mark CP13 complete.
- **Audio / touch** — MEMS analog microphone capture, speaker playback (PWM)
  and touch logo remain unattached as board-level live devices.
  The microphone uses P0.05/AIN3, not the nRF52 PDM peripheral; see the
  [foundation schematic pinmap](https://tech.microbit.org/hardware/schematic/).
- **Buttons A/B** — declared as `board_io` input stubs (active-low); no
  debounce, pull-up or interrupt wiring is modelled beyond the GPIO pin level.
- **Interface MCU (KL27/DAPLink)** — not modelled; the UART connector bridges
  `uart0` to the host console only.
- **Flash programming / NVMC erase-write cycles, UICR writes** — register
  models only; no endurance or lock semantics.
- **Silicon bench diff** — none exists for this part. There is no oracle for
  reset state, register values, timing or executing fidelity.

---

## Evidence and CI status

- Instruction audit (tier-1 fixture, micro:bit v2 system, 200 000 steps):
  **0 unknown Thumb-16 / 0 unhandled Thumb-32 / 0 unknown RISC-V, 100 %
  instruction coverage** — exact command and metrics in
  [`examples/microbit-v2/VALIDATION.md`](../../examples/microbit-v2/VALIDATION.md).
- Tier-1 fixture blob is pinned by sha256 in `tests/fixtures/tier1/MANIFEST.json`
  and rebuilt weekly by the `tier1-fixture-drift` job (`.github/workflows/core-nightly.yml`).
- Chip row is registered in `docs/coverage/tier1-matrix.json`; the tier-1
  matrix/ratchet tests (`cargo test -p labwired-cli --test tier1_matrix*`) gate
  pass→anything regressions.
- L1 smoke cell: `.github/workflows/core-coverage-matrix-smoke.yml` (microbit-v2).

**Scope statement.** L3 here means: the six rubric classes and the peripheral
checks listed above are proven for the documented scenarios on this chip model,
with a clean instruction audit and pinned artifacts. It does not mean the part
is silicon-equivalent, and firmware that depends on the unmodelled or
partially-modelled blocks above is not supported.
