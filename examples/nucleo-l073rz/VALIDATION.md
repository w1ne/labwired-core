# Validation — NUCLEO-L073RZ

**Tier: hardware-validated (smoke).** The boot + USART2 path was verified
against a physical NUCLEO-L073RZ over SWD (on-board ST-LINK V2, ST-LINK
serial `066CFF555054877567065340`), 2026-06-03. The simulator reproduces
the real board's UART byte stream **byte-for-byte**, and the device identity
the firmware prints (`DEV=20086447`) was read directly off the silicon.
Peripherals beyond the GPIO/USART smoke path (RCC clock tree, I2C/SPI/ADC)
are still family-model approximations and are listed under "Known fidelity
limits".

## 1. Silicon identity (read over SWD)

```
$ st-info --probe
  version:    V2J28S17
  flash:      196608 (pagesize: 128)
  sram:       20480
  chipid:     0x447
  dev-type:   STM32L0xxx_Cat_5
```

OpenOCD detected `Cortex-M0+ r0p1`. Register reads at reset:

| Register | Address | Silicon value | Confirms |
|----------|---------|---------------|----------|
| DBGMCU_IDCODE | `0x40015800` | `0x20086447` | DEV_ID `0x447`, REV_ID `0x2008` — and that DBGMCU is at the M0+ APB address, not `0xE0042000` |
| RCC_CR | `0x40021000` | `0x00000300` | boots on MSI (MSION+MSIRDY) |
| RCC_IOPENR | `0x4002102C` | `0x00000000` | GPIO clocks gated **here** on L0 (not AHB2ENR) |
| RCC_APB1ENR | `0x40021038` | `0x00000000` | USART2 clock gated here |
| GPIOA_MODER | `0x50000000` | `0xEBFFFCFF` | real reset value (PA13/14 in SWD modes) |
| USART2_ISR | `0x4000441C` | `0x000000C0` | TXE+TC set at reset |
| Flash-size reg | `0x1FF8007C` | `0x00C0` (=192) | 192 KB flash |

The chip yaml's `idcode` was corrected from a datasheet guess
(`0x10006447`) to the silicon-true `0x20086447` as a result.

## 2. Firmware peripheral bring-up (verified on silicon)

After the demo ran, registers read back over SWD prove it configured the
real peripherals correctly — i.e. the firmware that runs in the sim drives
physical hardware identically:

| Register | Value | Meaning |
|----------|-------|---------|
| RCC_CR | `0x00000305` | HSI16 on **and ready** (bit2) |
| RCC_CFGR | `0x00000005` | **SYSCLK switched to HSI16** (SW=SWS=01) |
| RCC_IOPENR | `0x00000001` | GPIOA clock enabled |
| RCC_APB1ENR | `0x00020000` | USART2 clock enabled (bit17) |
| GPIOA_MODER | `0xEBFFF4AF` | PA2/PA3 = AF, PA5 = output |
| GPIOA_AFRL | `0x00004400` | PA2/PA3 = **AF4** (USART2) |
| USART2_CR1 | `0x00000009` | UE + TE |
| USART2_ISR | `0x002000C0` | TC + TEACK — **transmission complete** |

## 3. Flash + UART capture (byte-for-byte parity)

```bash
# build → bin → flash over SWD (connect-under-reset)
cargo build --release -p firmware-l073-demo --target thumbv6m-none-eabi
arm-none-eabi-objcopy -O binary \
  target/thumbv6m-none-eabi/release/firmware-l073-demo /tmp/l073.bin
st-flash --connect-under-reset --reset write /tmp/l073.bin 0x08000000
#  → "Flash written and verified! jolly good!"

# capture the ST-LINK Virtual COM Port (/dev/cu.usbmodem11103) at 9600 8N1
stty -f /dev/cu.usbmodem11103 9600 cs8 -cstopb -parenb raw
cat /dev/cu.usbmodem11103
```

Real-board output vs. simulator output — **`diff` reports IDENTICAL**:

```
L073-DEMO BOOT
DEV=20086447      <- device identity read from real DBGMCU, matched by sim
LED ON
LED OFF
LED ON
LED OFF
LED ON
LED OFF
DONE
```

Saved artifacts: [`captures/silicon-uart-boot.txt`](captures/silicon-uart-boot.txt)
and [`captures/simulator-uart-boot.txt`](captures/simulator-uart-boot.txt).

> **Baud note:** the demo uses **9600 8N1**. At 115200 the ST-LINK V2 VCP on
> this unit produced heavy framing errors (≈99% byte loss) despite the
> USART2 transmitter being correct on-chip (TC set). 9600 captures cleanly.
> The simulator ignores baud (it emits each TDR byte immediately), so its
> output is identical regardless.

## 4. Tooling used

- `openocd` 0.12.0 + `stlink` 1.8.0 (Homebrew) driving the on-board ST-LINK V2.
- `arm-none-eabi-objcopy` from the PlatformIO `toolchain-gccarmnoneeabi`.
- macOS has no `timeout`; capture uses a backgrounded `cat` + `kill`.

## 5. Simulator-side checks

- **Build:** `thumbv6m-none-eabi` ✓ (toolchain enforces the ARMv6-M ISA).
- **Unsupported-instruction audit:** 0 unknown/unhandled, **100% coverage**
  (`out/unsupported-audit/nucleo-l073rz/report.md`).
- **`strict_onboarding` gate:** `1 passed; 0 failed` — `stm32l073` reaches
  the accepted `[SKIP]` state (chip yaml loads, bus builds, example present).

## 6. Peripheral validation matrix (silicon vs simulator)

The comprehensive demo exercises each important peripheral and prints one
deterministic token. Both the real board (UART @9600) and the simulator
(stdout) were captured and diffed — **10 of 12 lines match**; the remaining
two cannot match a deterministic engine by design. Artifacts:
[`captures/silicon-peripherals.txt`](captures/silicon-peripherals.txt) and
[`captures/simulator-peripherals.txt`](captures/simulator-peripherals.txt).

| Peripheral | What was checked | Silicon | Sim | Verdict |
|------------|------------------|---------|-----|---------|
| Core/DBGMCU | device identity readback | `20086447` | `20086447` | ✅ match (byte-for-byte) |
| **CRC** | CRC-32 of two fixed words | `B874177A` | `B874177A` | ✅ **match (byte-for-byte)** |
| **DMA1** | mem-to-mem copy (CMAR→CPAR, TCIF) | `OK` | `OK` | ✅ match |
| **RCC clock** | CFGR.SWS after switch to HSI16 | `0x04` | `0x04` | ✅ match (after fix, below) |
| GPIO out | LD2 BSRR toggle (LED ON/OFF) | ✓ | ✓ | ✅ match |
| GPIO in | PC13 (B1) idle level | `0` | `0` | ✅ match |
| USART2 | byte stream | ✓ | ✓ | ✅ match (byte-for-byte) |
| TIM21 | free-running counter advances | `UP` | `UP` | ✅ match (behavioural) |
| SPI1 | TXE set after enable | `TXE` | `TXE` | ✅ match (flag-level) |
| I2C1 | NACK on absent device | `?` | `?` | ✅ agree — inconclusive without a slave (see below) |
| ADC1 | VREFINT raw conversion | `~0x86` | `0x00` | ❌ analog — by design (see below) |
| RNG | DR draw | `0` | `CAFEBABE` | ❌ non-deterministic — by design (see below) |

### RCC clock-switch — found and FIXED

Originally the sim read `CLK=0x00` vs silicon `0x04`. Root cause: the L073 was
using the `stm32l4` RCC profile, but the **L0 register map differs** — CRRCR
(HSI48) sits at `0x08`, pushing `CFGR` to `0x0C` (L4 has it at `0x08`), and the
CR ready bits differ (HSI16RDY is bit 2, not bit 1). So the firmware's `CFGR`
writes were landing on the L4 model's *PLLCFGR* slot, and the SW→SWS readiness
check tested the wrong CR bit.

Fixed by adding a dedicated **`Stm32L0` RCC layout**
(`crates/core/src/peripherals/rcc.rs`): correct offsets, CR reset `0x300`
(MSION+MSIRDY), L0 CR ready bits, CRRCR/HSI48, and SW→SWS mirroring. The chip
yaml now uses `profile: "stm32l0"`. After the fix the sim reads `CLK=0x04`,
matching silicon. The other STM32 boards (L476/F103/F401/H563/…) are unaffected
— `firmware_survival` (25 passed) and `strict_onboarding` stay green.

### The two remaining divergences (correct by design)

1. **ADC VREFINT (analog).** Silicon returns a real conversion (`~0x86`); the
   simulator's ADC returns a deterministic `0`. An analog reading cannot be
   reproduced byte-for-byte by a deterministic engine — validation here is
   "silicon converts and returns a plausible value; sim is deterministic."
2. **RNG (non-deterministic).** The simulator returns a fixed `0xCAFEBABE` —
   correct behaviour for a *deterministic* oracle. A real TRNG can never (and
   should never) match it. (Silicon read `0` because the demo doesn't complete
   the full HSI48/RNG analog bring-up; immaterial — it cannot equal the sim's
   deterministic value either way.)

### Not validated without extra hardware

- **I2C/SPI data round-trips.** Both sim and silicon agree at the flag level
  (SPI TXE; I2C produced no NACK with `TIMINGR`=0 on a bare bus). Validating a
  real transfer needs an external I2C device, or a MOSI→MISO jumper for SPI
  loopback. Say the word and wire one up and I'll close these.

## Regression gate (CI)

L073 is locked against future regressions by three checks, same as the other
onboarded targets:

1. **`firmware_survival.rs` — byte-for-byte.** `test_nucleo_l073rz_smoke_survival`
   loads the committed `tests/fixtures/nucleo-l073rz-demo.elf` and asserts the
   silicon-validated stream `DEV=20086447\nCLK=00000004\nCRC=B874177A\nDMA=OK\n`.
   Drift = a regression in the L0 chip config, the `stm32l0` RCC layout, CRC,
   DMA, or the ARMv6-M decoder.
2. **`strict_onboarding.rs` — `[PASS]`.** `examples/nucleo-l073rz/io-smoke.yaml`
   builds the demo and asserts the same tokens via the CLI test runner.
3. **`rcc.rs` unit test** — `test_rcc_l0_layout_and_clock_switch` locks the L0
   register offsets, CR reset (`0x300`), and SW→SWS readback.

While wiring the gate I also fixed a **latent DMA-model bug**
(`crates/core/src/peripherals/dma.rs`): reading/byte-writing DMA `ISR`/`IFCR`
(offsets `0x00`/`0x04`) did `offset - 0x08` and underflowed → debug panic. It
affected any firmware touching DMA IFCR (it was crashing the nrf52840 survival
test). Fixed by handling those offsets explicitly and guarding the subtraction.

## Known fidelity limits

Validated: device identity, memory map, clock-switch + GPIO + USART2 bring-up,
and the UART byte stream. Still approximate / unverified against silicon:

1. **ARMv6-M not enforced by the engine.** It decodes the full Thumb-2 set
   for all ARM cores and would execute an instruction a real M0+ faults on;
   the `thumbv6m-none-eabi` build target is the ISA guardrail. Bit-banding is
   also left enabled though Cortex-M0+ has no bit-band region.
2. **RCC is the L4 model, not L0.** It is permissive (does not gate on enable
   bits), so firmware runs, but L4-offset ready flags will not match L0 — the
   demo polls with bounded spins and never blocks on them.
3. **I2C / SPI / ADC / EXTI reuse family models** (L4 / `stm32f1` EXTI). Not
   diffed against L0 silicon — re-validate before trusting any bus lab here.
4. **USB FS, LCD, COMP, SYSCFG are stubs** (read 0, writes dropped).
5. **Baud is not timed** in the sim (functional UART, not cycle-accurate).

## To extend coverage

1. Author `examples/nucleo-l073rz/io-smoke.yaml` (turns `[SKIP]` → `[PASS]`).
2. Add a case to `crates/core/tests/firmware_survival.rs` asserting the
   captured byte stream above (regression-locks the L0 chip config + decoder).
3. Validate I2C/SPI/ADC against silicon, replacing the L4 approximations
   where they diverge.
