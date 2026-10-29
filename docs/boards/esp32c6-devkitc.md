# ESP32-C6-DevKitC-1

Espressif **ESP32-C6** on the **ESP32-C6-DevKitC-1** (v1.2) — single HP core **RISC-V RV32IMAC @ 160 MHz**, 512 KB HP SRAM, 320 KB ROM, 8 MB external SPI flash on the WROOM-1 module, plus an LP core and Wi-Fi 6 / Bluetooth 5 / IEEE 802.15.4 radios on silicon. LabWired's C6 target runs a bare-metal ELF whose console reaches the capture sink.

This is an **L3 (production-ready)** target by the [target support rubric](../target_support_rubric.md): the full twelve-cell tier-1 row — clock (PCR), GPIO, UART, timer (TIMG), DMA (GDMA), interrupt delivery, I²C, SPI, ADC, PWM (LEDC), watchdog (TIMG MWDT) and RTC (LP_TIMER) — is proven by the tier-1 fixture, and the L2 package (CI lanes, known-limitations file, unsupported-instruction audit) is in place. The C3-shared Espressif IP (UART, GPIO, I²C0, GP-SPI2, LEDC, APB_SARADC, TIMG) is reused where the C6 SVD register head is offset-identical; PCR, GDMA, the MWDT path and LP_TIMER are C6-specific models. No silicon capture exists: register behaviour is SIM-DERIVED.

!!! tip "Live status"
    The tables below are a maintained snapshot. Authoritative automation:

    - [Chip conformance scoreboard](../coverage/chip-conformance.md) — level · modelled peripherals · register-match
    - [Bus visibility scoreboard](../coverage/bus-visibility.md) — which buses produce decodable edges
    - [Target support rubric](../target_support_rubric.md) — modeled / stub / silicon-verified

---

## Status at a glance

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/esp32c6.yaml`](../../configs/chips/esp32c6.yaml) |
| Example system | [`configs/systems/esp32c6-devkitc.yaml`](../../configs/systems/esp32c6-devkitc.yaml) |
| Reference firmware | [`crates/firmware-esp32c6-demo/`](../../crates/firmware-esp32c6-demo/) |
| TIER1 fixture | [`examples/tier1-fixture/esp32c6/`](../../examples/tier1-fixture/esp32c6/) → `tests/fixtures/tier1/esp32c6.elf` |
| Example | [`examples/esp32c6-devkitc/`](../../examples/esp32c6-devkitc/) |
| Committed artifact | `tests/fixtures/esp32c6-demo.elf` (`OK\n`) |
| Register/interrupt source | Vendored `tests/fixtures/real_world/esp32c6.svd` (espressif/svd) |
| Tier (snapshot) | **L3** — SIM-DERIVED, no silicon diff. Tier-1 row: all 12 classes pass (`clock` `gpio` `uart` `timer` `dma` `irq` `i2c` `spi` `adc` `pwm` `wdt` `rtc`) |
| Known limitations | [`examples/esp32c6-devkitc/KNOWN_LIMITATIONS.md`](../../examples/esp32c6-devkitc/KNOWN_LIMITATIONS.md) |

---

## Flash / firmware artifact

| Use | Artifact | Notes |
|-----|----------|--------|
| **Bare / demo crate** | ELF linked into the C6's unified flash window | `cargo build -p firmware-esp32c6-demo --release --target riscv32imc-unknown-none-elf` |
| **ESP-IDF / esptool image** | Merged flash `.bin` (bootloader + partition table + app) | **Not modelled end-to-end.** The bootloader/ROM path and partition parsing are out of scope at L1 |

Unlike the C3, the C6 maps **both** instructions and constant data through the
same external-memory window (`0x4200_0000..0x42FF_FFFF`; esp-idf v5.3
`soc/esp32c6/include/soc/soc.h` sets `SOC_IROM_LOW = SOC_DROM_LOW = 0x42000000`).
The 8 MB DevKitC-1 flash is declared as that window and behaves as plain memory:
there is **no SPI flash controller** behind it, so a real IDF image's ROM boot
sequence cannot run. The smoke ELF is loaded straight at its link addresses.

**ROM region (optional):** 320 KB at `0x4000_0000`, loadable via
`LABWIRED_ESP32C6_ROM`. Nothing on the L1 smoke path needs it; without the env
var the window stays zero.

---

## Pins (ESP32-C6-DevKitC-1 v1.2)

| Board label | GPIO | Typical default | Notes |
|-------------|------|-----------------|--------|
| TX (J3-2) | GPIO16 | **U0TXD** | USB-to-UART console |
| RX (J3-3) | GPIO17 | **U0RXD** | USB-to-UART console |
| — | GPIO8 | **Addressable RGB LED** (WS2812) | Needs RMT frame; pin stub only |
| BOOT (J3-11) | GPIO9 | BOOT button (active low) | Strapping pin; download mode not modelled |
| D0… | GPIO0–GPIO7 | LP-capable / ADC1 / strapping (MTMS/MTDI) | Advertised via the shared GPIO matrix model |
| D1… | GPIO10, GPIO11, GPIO15, GPIO18–GPIO23 | Header pins | Not modelled beyond the GPIO window |
| 3V3 / 5V / GND | — | Power | Digital levels in sim — not SPICE |

The GPIO matrix reuses the engine's C3-sized register block (pins 0–25), so
**GPIO26–GPIO30 are not modelled**. C6 GPIO has 31 pins
(`SOC_GPIO_PIN_COUNT` = 31 in `soc_caps.h`).

---

## Support matrix

| Mark | Meaning |
|------|---------|
| ✅ | Modeled well enough for the documented L1 smoke |
| ⚠️ | Present but partial, stubbed, or easy to misuse |
| ❌ | Not simulated — use the bench |

### Core & boot

| Block | Status | Notes |
|-------|--------|-------|
| RV32IMAC HP core | ✅ / ⚠️ | Interpreter executes the smoke; the A-extension decodes but is not exercised by the `imc` fixture |
| Memory map (flash window, 512 KB HP SRAM, 320 KB ROM region, 16 KB LP SRAM) | ✅ | Bases match the C6 memory map and `soc.h` |
| Mask ROM | ⚠️ | Optional image via `LABWIRED_ESP32C6_ROM`; not the reset path of this ELF |
| Reset / clocks | ✅ / ⚠️ | PCR is a native register file (full SVD map) and the `clock:` gate controller: `CLK_EN` is **enforced** by the shared bus gate for UART0/UART1/TIMG0/TIMG1/GDMA/I²C0/SPI2/LEDC/SARADC. `RST_EN` is recorded, not enforced; no clock tree (dividers are storage) |
| LP core (RV32IMC) | ❌ | Not modelled |

### Console, GPIO, IO_MUX

| Block | Status | Notes |
|-------|--------|-------|
| UART0 / UART1 | ✅ / ⚠️ | Espressif UART IP + 128-byte FIFOs, clocked at 160 MHz (`esp32c6_uart`). Head map only: the C6 register tail (`SLEEP_CONF*`, `CLK_CONF` @0x88, `DATE` @0x8C, `ID` @0x9C) is unmodelled and reads as the C3's old offsets |
| GPIO matrix | ⚠️ | Shared C3-sized model (26 pins); `board_io` LED/button declared but the RGB LED needs RMT |
| IO_MUX | ⚠️ | Register-backed stub: pad function writes are recorded, not electrically enforced — the console shifts bytes regardless of routing |
| USB Serial/JTAG | ❌ | `USB_DEVICE` window deliberately unmapped |
| I²C0 | ✅ / ⚠️ | Behavioral command-list engine shared with the C3 (`esp32c3_i2c`), C6 base `0x6000_4000` + source 50 (I2C_EXT0). The C6-only differences (SDA/SCL filter tail) are unmodelled; the C3 matrix-signal pad wiring is applied by the shared wiring pass even though C6 signal ids differ |
| GP-SPI2 | ✅ / ⚠️ | Behavioral GP-SPI transaction engine shared with the C3 (`esp32c3_spi`), C6 base `0x6008_1000` + source 72. No external-device/CS-gating or DMA-coupled path |
| APB_SARADC | ✅ / ⚠️ | Behavioral one-shot engine shared with the C3 (`esp32c3_apb_saradc`), C6 base `0x6000_E000` + source 60. Register reset seeds are the C3 silicon capture; no DMA mode, thresholds or TSENS |
| LEDC | ✅ / ⚠️ | Live timer/counter/wrap engine shared with the C3 (`esp32c3_ledc`), C6 base `0x6000_7000` + source 45. The C6-only gamma/event/compare/capture tail is not modelled; no pad-drive claim |
| TIMG0/TIMG1 | ✅ / ⚠️ | Shared `esp32_timg` GP timer plus the real C3/C6 **MWDT** path (`esp32c6_mwdt`): WDTWPROTECT write lock, `WDTCONFIG0..5` round-trip, walk-driven **four-stage chain** (`0→1→2→3→0`, WDTCONFIG2..5 stage holds, STG0..3 interrupt-action encodings), WDTFEED reload to stage 0, `INT_RAW_TIMERS.WDT_INT_RAW` latch + W1C. The CPU/system reset actions are **not** performed (advanced through, never triggered) |
| LP_TIMER (RTC) | ✅ / ⚠️ | C6-only `esp32c6_lp_rtc`: free-running 48-bit counter + `UPDATE` snapshot into `MAIN_BUF0`/`MAIN_BUF1`. No RTC-slow rate, alarms/overflow IRQs, or sleep retention |
| SYSTIMER, RMT, I2S, TWAI, crypto, eFuse, PMU, LP_WDT, LP_I2C/LP_UART, LP core | ❌ | Deliberately **not declared**; their windows fault loudly instead of pretending to work |

### Interrupts

| Block | Status | Notes |
|-------|--------|-------|
| Interrupt matrix (`INTERRUPT_CORE0`, base 0x6001_0000) | ✅ (software + peripheral sources) | SVD-derived `declarative` MAP bank, **honoured by a C6 interrupt fabric** (`crates/core/src/bus/routing.rs`, C6 INTPRI layout arm): asserted sources route through the MAP word, enable, priority and threshold gates into the RISC-V core's external lines. Proven end to end by the TIER1 fixture for BOTH source kinds — `CPU_INTR_FROM_CPU_0` (source 22, MAP `@0x6001_0058`) rings a real `mcause=0x8000_0009` trap, and **UART0's peripheral source 43** (its `irq: 43` descriptor wiring through the shared Espressif twin) maps to line 10 and takes a real `mcause=0x8000_000A` trap on `INT_ENA.TX_DONE`, with `UART_INT_CLR` acknowledged and the line proven de-asserted by re-enable. I²C0=50/SPI2=72/LEDC=45/SARADC=60/GDMA=66–71 sources are declared and fed to the models, but no fixture takes a trap from them; the C6 GPIO model does not emit a GPIO matrix-interrupt source |
| INTPRI (`0x600C_5000`) | ✅ (register model) | SVD-derived `declarative` block (`configs/peripherals/esp32c6/intpri.yaml`): `CPU_INT_ENABLE` @0x00, `CPU_INT_PRI_n` @0x0C+n*4, `CPU_INT_THRESH` @0x8C, `CPU_INTR_FROM_CPU_n` @0x90+n*4. The C3 splits these between `INTERRUPT_CORE0` and `SYSTEM`; declaring the C6 block is what arms matrix routing |
| GPIO IRQ 30 | ⚠️ | Declared in the source SVD; not routed (no GPIO interrupt generator in the shared C3 GPIO model) |

### Radio & network

| Block | Status | Notes |
|-------|--------|-------|
| Wi-Fi 6 (2.4 GHz), Bluetooth 5 (LE), IEEE 802.15.4 (Zigbee/Thread) | ❌ | **Not modelled at all** — no MAC, no PHY, no packet path. Do not run networking firmware here |
| LP peripherals / deep sleep | ❌ | Not modelled |

---

## Tier-1 peripheral matrix (as of 2026-09-21)

The chip has a Tier-1 row via
[`examples/tier1-fixture/esp32c6/`](../../examples/tier1-fixture/esp32c6/)
(`tests/fixtures/tier1/esp32c6.elf`), a bare-metal `no_std` firmware that
pokes raw MMIO and reports the `TIER1 <class> PASS|FAIL` protocol over UART0.
Classes with no declared peripheral type stay `na` in
[`docs/coverage/tier1-matrix.json`](../coverage/tier1-matrix.json).

| Class | Cell | Evidence / why |
|-------|------|----------------|
| uart | **pass** | implicit — the `TIER1` transcript arrives over UART0 (Espressif twin, 128-byte FIFO) |
| gpio | **pass** | `ENABLE`/`OUT` stores plus real `W1TS`/`W1TC` set/clear side effects read back through `OUT`/`ENABLE`; `FUNCn_OUT_SEL_CFG` / `FUNCn_IN_SEL_CFG` words round-trip; `IN` does not follow the output latch |
| irq | **pass** | real CPU traps from BOTH source kinds. Software: `CPU_INTR_FROM_CPU_0` (matrix source 22) → MAP → enabled line 9 → `mcause=0x8000_0009`, handler runs and acknowledges; disabling the line proves the enable gate masks a second doorbell. Peripheral: UART0 (`irq: 43` descriptor wiring, `esp32c6_uart` → shared Espressif twin) armed with `INT_ENA.TX_DONE`, one byte shifted out so the latched `TX_DONE` raw rises → MAP → enabled line 10 → `mcause=0x8000_000A`; the handler's `UART_INT_CLR` W1C ack clears the source, the main flow re-enables line 10 and requires no second trap |
| clock | **pass** | `pcr` (native `esp32c6_pcr`, full SVD map) is the yaml `clock:` gate controller: `UART0_SCLK_CONF`/`SYSCLK_CONF` round-trip, and `UART0_CONF.CLK_EN=0` really silences UART0 (`reads → 0`, writes dropped, pre-gate value survives); `CLK_EN=1` restores it. Same gate declared for UART1/TIMG0/TIMG1/GDMA/I²C0/SPI2/LEDC/SARADC. `RST_EN` is recorded, not enforced |
| timer | **pass** | TIMG0 (`esp32c6_mwdt` = shared `esp32::timg::Timg` + MWDT): `T0CONFIG.EN` → `T0UPDATE`-latched `T0LO/T0HI` advances across a bounded spin; clearing `EN` freezes the counter |
| dma | **pass** | GDMA (0x6008_0000, `esp32c6_gdma`, 3 channels): real in-RAM linked-list mem→mem transfer; descriptors walked, bytes land in the destination, `IN_SUC_EOF`/`IN_DONE` + `OUT_TOTAL_EOF`/`OUT_DONE` latch, owner bits written back (`OUT_AUTO_WRBACK`) |
| i2c | **pass** | I²C0 (0x6000_4000, `esp32c3_i2c` with C6 source 50): RSTART→WRITE(1)→STOP walks to STOP, `COMD.command_done` latches, `CTR.TRANS_START` self-clears, `INT_RAW.TRANS_COMPLETE` latches after the wire transaction (NACKed address, no slave attached). Matrix-routed pad proof: `esp32c6_i2c_waveform` (signals 45/46, pads 16/17) |
| spi | **pass** | GP-SPI2 (0x6008_1000, `esp32c3_spi` with C6 source 72): `SPI_CMD.USR` launch self-clears, `TRANS_DONE` latches in `DMA_INT_RAW`, and `W0` reads back `0xFFFF_FFFF` (idle-bus MISO shifted in) |
| adc | **pass** | APB_SARADC (0x6000_E000, `esp32c3_apb_saradc` with C6 source 60): one-shot `ONETIME_START` self-clears, `SAR1_DONE` latches, `SAR1DATA_STATUS` packs a channel-dependent 12-bit sample plus the selected channel id; two channels differ predictably |
| pwm | **pass** | LEDC (0x6000_7000, `esp32c3_ledc` with C6 source 45): TIMER0's live counter advances, wraps at `2^DUTY_RES` and latches `LSTIMER0_OVF`; `PAUSE` freezes it and stops new overflows |
| wdt | **pass** | TIMG0 MWDT: `WDTWPROTECT` resets to the key (unlocked); locking it makes `WDTCONFIG0..5` writes drop (readback unchanged) while `WDTFEED` stays writable; `WDTCONFIG1` round-trips; with `STG0_HOLD` programmed, the walk-driven stage-0 countdown latches `INT_RAW_TIMERS.WDT_INT_RAW` and `INT_CLR_TIMERS` clears it W1C. The four-stage chain is real: with STG1 also an interrupt stage, a second latch arrives WITHOUT a feed (only the chain advancing into stage 1 can produce it), and another feed restarts stage 0 (the model is in stage 2 by then, whose seeded SVD hold is ~1M ticks, so the latch proves the stage pointer reset). The CPU/system reset actions (STG=2/3) are NOT performed — no safe bus reset-request path exists |
| rtc | **pass** | LP_TIMER (0x600B_0C00, `esp32c6_lp_rtc`): the `UPDATE` bit-28 strobe latches the live 48-bit counter into `MAIN_BUF0`, the readout stays frozen without a new strobe, a second strobe after elapsed cycles is strictly greater and shifts the old snapshot into `MAIN_BUF1`. No RTC-slow rate claim |

Observed transcript (`--max-steps 8000000`):

```text
TIER1 clock PASS
TIER1 gpio PASS
TIER1 timer PASS
TIER1 dma PASS
TIER1 irq PASS
TIER1 i2c PASS
TIER1 spi PASS
TIER1 adc PASS
TIER1 pwm PASS
TIER1 wdt PASS
TIER1 rtc PASS
TIER1 done
```

**Not proven / known gaps**

- `irq` proves the software (doorbell) path and the UART0 peripheral path
  (`TX_DONE` → source 43 → line 10 → trap → W1C ack → re-enable proof). It does
  **not** prove GPIO interrupt delivery, and the other peripheral classes'
  sources (I²C0=50, SPI2=72, LEDC=45, SARADC=60, GDMA=66–71, LP_TIMER=15) are
  declared and fed to the models, but the fixture polls RAW status rather than
  taking a trap from them. A scheduler-driven peripheral's routed level
  de-assert is re-derived at the peripheral tick (≤ one tick interval), not at
  the peripheral's MMIO write — the fixture's ISR masks its line before `mret`
  for that reason and proves the real de-assert by re-enabling it.
- `clock` proves `CLK_EN` gating for the nine declared gated peripherals; the
  clock tree behind the PCR dividers (actual frequencies) is not modelled, and
  `RST_EN` reset semantics are not enforced.
- `dma` proves the memory-to-memory GDMA descriptor path. Peripheral-coupled
  DMA (SPI/UART/I2S/AES/…) is unimplemented and stalls visibly.
- The GPIO claim is register/side-effect behaviour on the shared C3-sized
  model. IO_MUX pad routing stays declarative: no electrical pad claim. The
  I²C0 matrix-routed pads are proven on the C6 (signals 45/46; see
  `esp32c6_i2c_waveform`); SPI2 pad routing on the C6 is still unproven.
- `i2c`/`spi`/`adc`/`pwm` reuse the C3 behavioral engines at the C6 bases; the
  C6 register tails those engines do not touch (LEDC gamma/capture/event,
  I²C filter tail, SARADC TSENS/CALI) are register-backed storage only.
- `wdt` proves the full stage chain's interrupt-action path (stages 0 and 1 in
  the fixture; all four in the unit suite): stage holds drive the chain
  `0→1→2→3→0`, every interrupt-action stage latches, a feed returns to stage 0,
  and off/reset-action stages still advance. Still **not** modelled: the
  CPU/system reset actions (no reset is ever performed), the 12.5 ns ×
  prescaler timeout rate (the countdown is walk-driven; the hold counts
  peripheral walk ticks, and WDTCONFIG1's prescaler does not scale it), and
  interrupt delivery of the latch through the matrix (`TG0_WDT_LEVEL`=53 is
  declared but not routed).
- `rtc` proves the LP_TIMER snapshot protocol only: no RTC-slow rate,
  TAR0/TAR1 alarm comparators, overflow/wakeup interrupts, or sleep retention.
- The interrupt fabric reuses the C3 engine (`ESP32c3Fabric`) with a C6
  register layout selected by the `INTPRI` block's presence. Differences from
  silicon that remain: no CLIC/`CPU_INT_TYPE` edge-vs-level programming, no
  `CPU_INT_CLEAR` write path, and no U-mode/privilege handling.
- ROM boot, the LP core, and every radio remain unmodelled. The full list is
  maintained in [`examples/esp32c6-devkitc/KNOWN_LIMITATIONS.md`](../../examples/esp32c6-devkitc/KNOWN_LIMITATIONS.md).

---

## What it catches vs what needs a bench

**Sim is strong for:** CPU/decode regressions on a second RISC-V memory map;
UART bring-up sequencing (PCR gate → IO_MUX route → CLKDIV → FIFO); register-map
and IRQ cross-checks against the vendor SVD; interrupt-matrix routing and real
RISC-V trap delivery from both the software doorbell and a peripheral source
(UART0 `TX_DONE`); the C3-shared Espressif IP at C6 bases (I²C command lists,
GP-SPI transactions, one-shot SAR conversions, LEDC timer/wrap/pause, TIMG
counters, the MWDT write-lock/feed/stage-chain contract, LP_TIMER snapshots);
deterministic CI of a C6 image.

**Still use silicon for:** the ROM bootloader and flash/partition images,
peripheral sources beyond UART0 (GPIO edges, I²C/SPI/ADC/LEDC/GDMA matrix
lines, edge-vs-level `CPU_INT_TYPE`, `CPU_INT_CLEAR` semantics, sub-tick
de-assert latency), watchdogs that must actually reset the chip, RTC wall-time
and sleep retention, matrix-routed SPI waveform fidelity on C6 pads, LP
core and power management, Wi-Fi/BT/802.15.4, USB Serial/JTAG, analog, power,
antenna/EMI, production sign-off.

---

## How to run

### CLI (oracle / CI)

```bash
cargo build -p firmware-esp32c6-demo --release --target riscv32imc-unknown-none-elf

cargo run -q -p labwired-cli -- \
  --firmware tests/fixtures/esp32c6-demo.elf \
  --system configs/systems/esp32c6-devkitc.yaml \
  --max-steps 200000
```

Expected: `OK` on the console, PC ending inside `0x4200_0000..0x42FF_FFFF`.

### Tier-1 fixture

```bash
scripts/tier1/build_esp32c6.sh

./target/debug/labwired run \
  --chip configs/chips/esp32c6.yaml \
  --firmware tests/fixtures/tier1/esp32c6.elf \
  --max-steps 8000000 2>&1 | grep -a TIER1
```

Expected transcript: `clock`, `gpio`, `timer`, `dma`, `irq`, `i2c`, `spi`, `adc`, `pwm`, `wdt`, `rtc` each `PASS`, then `TIER1 done`.

Instruction audit (L2 evidence): `./scripts/unsupported_instruction_audit.sh --firmware tests/fixtures/tier1/esp32c6.elf --system configs/systems/esp32c6-devkitc.yaml --max-steps 200000 --out-dir out/unsupported-audit/esp32c6-devkitc` → `unknown_riscv: 0`, `unsupported_total: 0`.

The runnable example (and its assertions) is
[`examples/esp32c6-devkitc/`](../../examples/esp32c6-devkitc/README.md).

### Playground

Not yet a catalog board — the L1 target is CLI/CI only until a differential and
a playground registration land.

### Agent (MCP)

Not yet registered.

---

## Related systems & examples

| Path | What |
|------|------|
| `configs/chips/esp32c6.yaml` | Chip descriptor (bases/IRQs cited) |
| `configs/systems/esp32c6-devkitc.yaml` | Baseline system |
| `examples/esp32c6-devkitc/` | Smoke example + validation runbook |
| `examples/tier1-fixture/esp32c6/` | Tier-1 raw-register fixture (all 12 classes: clock + gpio + timer + dma + irq + i2c + spi + adc + pwm + wdt + rtc + implicit uart) |
| `scripts/tier1/build_esp32c6.sh` | Builds/installs `tests/fixtures/tier1/esp32c6.elf` |
| `tests/fixtures/real_world/esp32c6.svd` | Vendored vendor SVD used by the gates |
| [ESP32-C3 board page](esp32c3.md) | Sibling Espressif RISC-V target (different map, deeper model) |
