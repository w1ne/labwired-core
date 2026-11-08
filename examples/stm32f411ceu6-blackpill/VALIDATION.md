# STM32F411CEU6 Black Pill Validation Runbook

Run all commands from `core/`.

## Physical scope — there is none

**No STM32F411 part was connected for this onboarding.** There is no ST-Link
capture, no SWD register diff, and no UART byte-parity check against silicon.
Every `PASS` below is simulation evidence produced by the deterministic engine
against a descriptor built from ST's own sources. The board's tier in
`validation/manifest.yaml` is `sim-validated` for exactly that reason.

## Modelled chip scope

The descriptor covers the STM32F411xC/xE peripheral memory map used by the
STM32F411CEU6: 512 KiB flash at `0x08000000`, 128 KiB SRAM at `0x20000000`,
GPIO ports A/B/C/D/E/H (no F/G — the RCC_AHB1ENR enable-bit gap at 5/6 is real).

Executable models: RCC (`stm32f4` profile), GPIO (`stm32v2`), SysTick,
USART1/2/6, I2C1/2/3, SPI1/2/3/4/5, TIM1/2/3/4/5/9/10/11 (TIM2 and TIM5 32-bit),
ADC1, IWDG, RTC, PWR (`stm32f4`), FLASH interface (`stm32f4`), EXTI.

Stubs (address windows present so firmware can probe them, no behaviour):
DMA1/DMA2, WWDG, SDIO, SYSCFG, CRC, the I2S extension windows, the USB OTG FS
windows, and DBGMCU.

Clock gating is declared for TIM2, ADC1, SPI1 and SPI5 only — the four gates the
tier-1 fixture actually proves. Every other block is reachable without setting
its RCC enable bit, which is **not** silicon behaviour; it is the same modelling
gap the F401 descriptors carry, and the verified enable bits for closing it are
listed in the chip yaml.

`ADC_Common` is deliberately not a separate window: it nests inside the ADC1
range and the descriptor validator rejects overlapping regions. The schema's
single-IRQ field also means TIM1/TIM9/TIM10/TIM11 are modelled for register
access and ticking without claiming full shared-IRQ fidelity.

## Sourced board surface and open points

### DBGMCU IDCODE

The F411 `DEV_ID`/`REV_ID` is in neither the SVD nor the CMSIS header. It needs
a real part read over SWD. The chip yaml therefore ships `dbg` as a **stub with
no IDCODE**, and no value is invented. The F401CDU6 descriptor's
`0x10016433` is **F401's** and must not be copied across — anything that
identifies a target by IDCODE will be wrong if you do.

### PC13 user LED

WeAct's MiniSTM32F4x1 product literature identifies the C13 LED, and the
LaskaKit listing for the WeAct STM32F411CEU6 board says its user LED is on
PC13. The linked V3.0 schematic connects the LED/resistor between 3.3 V and
PC13, which makes it active-low. The board profile and smoke expose only that
LED. This is source-supported mapping, not physical confirmation.

### PA0 user key

WeAct/LaskaKit documentation identifies the user key on PA0, but this board
descriptor leaves it out because the pressed logic has not been checked against
the switch net or on physical hardware.

### Max SYSCLK

The F411 runs to 100 MHz (vs the F401's 84). This is **unrepresentable** in the
current config schema — the `stm32f4` RCC model stores CR/CFGR/PLLCFGR as gated
bit-fields and derives no frequency at all. It is documentation only today and
has no functional effect; do not read a timing claim into the model.

## 1) Optional: ensure the target is installed

```bash
rustup target add thumbv7em-none-eabi
```

## 2) Validate the manifests

```bash
cargo run -q -p labwired-cli -- asset validate --chip configs/chips/stm32f411ceu6.yaml
cargo run -q -p labwired-cli -- asset validate --system configs/systems/stm32f411ceu6-blackpill.yaml
cargo run -q -p labwired-cli -- asset validate --chip configs/chips/stm32f411ceu6-blackpill-led-smoke.yaml
cargo run -q -p labwired-cli -- asset validate --system configs/systems/stm32f411ceu6-blackpill-led-smoke.yaml
```

Pass criteria: all four commands exit `0` with `"valid": true` and zero
errors.

## 3) Rebuild the tier-1 fixture blob (optional — a built blob is committed)

```bash
scripts/tier1/build_stm32.sh          # builds every STM32 fixture, incl. stm32f411
# or just this one:
cd examples/tier1-fixture/stm32f411 && cargo build --release --target thumbv7em-none-eabi
```

The blob lives at `tests/fixtures/tier1/stm32f411.elf` and its sha256 is pinned
in `tests/fixtures/tier1/MANIFEST.json`; update that hash if you rebuild.

## 4) Run the PC13 board LED smoke

This target uses a separate narrow descriptor. It declares RCC and GPIOC as
models and omits the unrelated chip peripherals and all stubs.

```bash
scripts/onboarding_smoke.sh \
  --target-id stm32f411ceu6-blackpill-led-smoke \
  --crate firmware-stm32f411-blackpill-smoke \
  --target thumbv7em-none-eabi \
  --script examples/stm32f411ceu6-blackpill/led-onboarding-smoke.yaml \
  --system configs/systems/stm32f411ceu6-blackpill-led-smoke.yaml \
  --out-dir examples/stm32f411ceu6-blackpill/evidence/led-smoke
```

Pass criteria: `run_smoke.log` says `PASS`, `result.json` records at least 16
edges on `gpioc:13`, and the stop reason is `max_steps`.

The hosted LabWired oracle also returned `model_verified` for PC13 with 48
transitions. Its power-rail approximation and the C translation used because
the hosted Rust scaffold could not link the interrupt table are recorded in
[`evidence/labwired-verification.md`](evidence/labwired-verification.md).
The local simulator likewise reports `unpowered_rail_assumed`: it supplies an
ideal rail and does not model power-on or brown-out reset. Neither run is
physical hardware evidence.

## 5) Run the broader F411 chip self-test

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/stm32f411ceu6-blackpill/io-smoke.yaml \
  --output-dir out/stm32f411ceu6-blackpill/io-smoke \
  --no-uart-stdout
```

Pass criteria:

1. exit code is `0`
2. `uart.log` contains, in order:
   `TIER1 clock PASS`, `TIER1 gpio PASS`, `TIER1 timer PASS`, `TIER1 i2c PASS`,
   `TIER1 spi PASS`, `TIER1 adc PASS`, `TIER1 wdt PASS`, `TIER1 rtc PASS`,
   `TIER1 done`
3. `result.json` reports `status: "pass"` and stop reason `max_steps`

`TIER1 done` arriving over USART2 is itself the proof of a working UART path —
that is why there is no separate `uart` line.

`TIER1 spi PASS` covers **both** SPI1 and SPI5. SPI5 is the F411's only extra
peripheral instance, and its clock gate (RCC_APB2ENR bit 20) is declared in no
public F411 SVD, so this assertion is the only executable check on that bit.

## 6) Direct JSON/VCD simulation

```bash
cargo run -q -p labwired-cli -- \
  --firmware tests/fixtures/tier1/stm32f411.elf \
  --system configs/systems/stm32f411ceu6-blackpill.yaml \
  --max-steps 5000000 \
  --json \
  --vcd out/stm32f411ceu6-blackpill/blackpill.vcd
```

Pass criteria: JSON reports `status: "finished"` and the VCD exists.

## 7) What would raise the tier

- Read the DBGMCU IDCODE off a real F411 over SWD and pin it in the chip yaml.
- Confirm the LED electrical behavior and PA0 pressed logic on a physical WeAct
  F411 board.
- A register reset-value capture (`reg_oracle.json`) diffed against the sim,
  which would let `chip_conformance` carry a `reset_oracle` for this chip.
- An executing-fidelity test (walk-vs-scheduler differential or a silicon
  exec-oracle) — the missing class that keeps this chip in `NOT_SHIPPED` in
  `board_coverage_ratchet.rs`.
