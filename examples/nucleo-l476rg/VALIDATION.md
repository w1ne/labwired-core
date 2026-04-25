# NUCLEO-L476RG hardware-validation log

Every commit to the L476 chip yaml or any peripheral that L476 firmware
touches must keep the survival tests green. This file is the audit
trail: which traces have been captured against real silicon, what
revealed each bug, and which simulator commits closed each gap.

## Hardware

- Board: **NUCLEO-L476RG** (ST production part, no rework)
- Debugger: on-board ST-LINK V2-1, currently flashed with **J-Link OB**
  firmware (`Firmware: J-Link STLink V21 compiled Aug 12 2019`).
  Compatible with both `JLinkExe` and (after re-flash) `st-flash`.
- Host: Linux, `arm-none-eabi-gcc 14.2.1`, OpenOCD 0.12.0+dev.
- DBGMCU IDCODE @ 0xE0042000 = `0x10076415` (DEV_ID 0x415, REV_ID 0x1007).

## Survival traces

Each row is a captured byte stream from `/dev/ttyACM1` at 115200 8N1.
Sim must reproduce verbatim (`crates/core/tests/firmware_survival.rs`).

| Trace                       | Fixture ELF                                   | Hardware capture file                                          |
|-----------------------------|-----------------------------------------------|----------------------------------------------------------------|
| `nucleo_l476rg_smoke`       | `tests/fixtures/nucleo-l476rg-smoke.elf`      | `tests/fixtures/hw_traces/nucleo_l476rg_smoke.txt`             |
| `nucleo_l476rg_spi`         | `tests/fixtures/nucleo-l476rg-spi.elf`        | `tests/fixtures/hw_traces/nucleo_l476rg_spi.txt`               |
| `nucleo_l476rg_i2c`         | `tests/fixtures/nucleo-l476rg-i2c.elf`        | `tests/fixtures/hw_traces/nucleo_l476rg_i2c.txt`               |
| `nucleo_l476rg_adc`         | `tests/fixtures/nucleo-l476rg-adc.elf`        | `tests/fixtures/hw_traces/nucleo_l476rg_adc.txt`               |
| `nucleo_l476rg_dma`         | `tests/fixtures/nucleo-l476rg-dma.elf`        | `tests/fixtures/hw_traces/nucleo_l476rg_dma.txt`               |
| `nucleo_l476rg_demo`        | `tests/fixtures/nucleo-l476rg-demo.elf`       | (built from `crates/firmware-l476-demo`, same trace as sim)    |
| `nucleo_l476rg_l4periphs`   | `tests/fixtures/nucleo-l476rg-l4periphs.elf`  | `tests/fixtures/hw_traces/nucleo_l476rg_l4periphs.txt`         |
| `nucleo_l476rg_l4periphs2`  | `tests/fixtures/nucleo-l476rg-l4periphs2.elf` | `tests/fixtures/hw_traces/nucleo_l476rg_l4periphs2.txt`        |
| `nucleo_l476rg_cubemx_hal`  | `tests/fixtures/nucleo-l476rg-cubemx-hal.elf` | (sim baseline — round 9; HW UART verified via JLink probe)     |

## Bugs surfaced and fixed

Each round captured a divergence between sim and silicon and patched
the simulator. Order matters — earlier rounds unblocked later ones.

### Round 1 — UART smoke (`nucleo_l476rg_smoke`)
- **Thumb-2 shift-by-register decoder** read shift_type from `h2[5:4]`
  (= 0) instead of `h1[6:5]`. Every `LSR.W`/`ASR.W`/`ROR.W` was being
  silently decoded as `LSL.W`. Surfaced via the stock GCC hex32 print
  loop emitting `lsr.w r2, r0, r3` (= `FA20 F203`).
- **Plain 12-bit ADDW (T4) / SUBW (T4)** were missing from the decoder.
  Fell through to Unknown32 → no-op. Added `AddwImm` / `SubwImm`
  variants with executor handlers.
- **DBGMCU peripheral** missing entirely. Reads at `0xE0042000`
  bus-faulted. Added a minimal `dbgmcu` peripheral with configurable
  IDCODE (set to `0x10076415` in `stm32l476.yaml`).
- **VFPv4 single-precision FPU** unimplemented. Every `VLDR/VSTR/VMUL/
  VADD/VSUB/VDIV/VMOV` returned Unknown32. Added `fpu_s: [u32; 32]` to
  CortexM and full decode + execute paths for the common subset; float
  math goes through Rust's `f32` so IEEE-754 binary32 matches silicon.

### Round 2 — SPI register fidelity (`nucleo_l476rg_spi`)
- **`SXTH` / `SXTB` / `UXTH`** missing — only `UXTB` was decoded. GCC
  emits `UXTH` for any `uint16_t -> u32` conversion in printf-style hex
  formatters. Decoder mask `0xFFC0` widened to `0xFF00`.
- **SPI CR2 reset value** was 0x0000; real STM32L4 resets to 0x0700
  (DS = 8-bit data size).
- **SPI auto-loopback** removed: sim was setting RXNE and writing the
  TX byte back into DR after every transmit. Real silicon with no
  slave wired leaves SR=0x0002 / DR=0x0000.

### Round 3 — I²C modern layout (`nucleo_l476rg_i2c`)
- **STM32L4 I²C register layout** added as `I2cRegisterLayout::Stm32L4`.
  Storage promoted u16 → u32. Adds TIMINGR/ISR/ICR/RXDR/TXDR; removes
  the F1-only CCR/TRISE/SR1/SR2/DR. Bit semantics:
  - ISR resets to `0x00000001` (TXE=1).
  - CR2.START set lights ISR.BUSY (bit 15); CR2.STOP clears it.
  - ICR is W1C — writing a 1 clears the corresponding ISR flag.
  - Writing TXDR clears ISR.TXE / ISR.TXIS.

### Round 4 — ADC modern layout (`nucleo_l476rg_adc`)
- **STM32L4 ADC register layout** added as `AdcRegisterLayout::Stm32L4`.
  ISR/IER/CR/CFGR/CFGR2/SMPR1-2/SQR1-4/DR @ 0x40 plus common block at
  0x300. Reset:
  - CR = `0x20000000` (DEEPPWD set — chip starts in deep-power-down).
  - CFGR = `0x80000000` (JQDIS = 1).
- **ADCAL latch**: ADCAL stays set forever when no ADC clock is sourced
  (firmware enables AHB2.ADCEN but not CCIPR.ADCSEL). Sim now matches.

### Round 5 — DMA + NVIC routing (`nucleo_l476rg_dma`)
- **CPAR/CMAR mutated during transfer** — sim was post-incrementing the
  user-facing register every element. Real DMA uses an internal pointer
  and leaves the configured base addresses readable. Added
  `cpar_ptr`/`cmar_ptr` internal fields.
- **ISR missing GIF / HTIF flags** — sim only set TCIF. Real hardware
  emits GIF (logical-OR of per-channel flags) and HTIF when CNDTR
  crosses half its initial value.
- **Peripheral IRQ < 16 routing trampled SVCall** — the bus had a
  special case `if irq >= 16` route to NVIC, else push as system
  exception. DMA1_CH1 (NVIC IRQ 11) was firing the SVCall vector when
  TCIE was enabled. Fix: SysTick now uses `system_exception` field on
  `PeripheralTickResult`; the NVIC IRQ path always goes through NVIC.
- **MEM2MEM data direction** — sim was copying CPAR → CMAR; real
  STM32 silicon does CMAR → CPAR when DIR=1 + MEM2MEM=1 (RM0351
  §11.4.7). Surfaced only when a self-test verified the destination
  word post-transfer.

### Round 6 — Foundational L4 peripherals (`nucleo_l476rg_l4periphs`)
- **PWR peripheral** added. STM32L4 reset values verified against
  silicon: CR1=0x00000200 (VOS=01, range 1), CR3=0x00008000 (EIWUL),
  SR2=0x00000100 (REGLPF). Required for HAL-generated firmware —
  HAL_PWREx_ControlVoltageScaling() runs before any RCC PLL config.
- **FLASH peripheral** added. ACR/KEYR/OPTKEYR/SR/CR/OPTR with the
  L4 reset values (ACR=0x00000600 — caches enabled by boot ROM,
  CR=0xC0000000 LOCK+OPTLOCK, OPTR=0xFFEFF8AA factory-programmed).
  KEYR/OPTKEYR walk the unlock state machine (write 0x45670123 then
  0xCDEF89AB to clear LOCK in CR). Without this, HAL latency adjustment
  before PLL switch silently no-ops and SYSCLK stays on MSI.
- **TIM2/TIM5 32-bit width** — existing `Timer` peripheral was 16-bit
  only. Added `width` config knob; TIM2/TIM5 on L4 are 32-bit, so ARR
  resets to 0xFFFFFFFF and CNT/ARR reads/writes use the full u32.
- **RNG peripheral** added with deterministic xorshift32 LFSR so
  firmware that seeds Rust's stdlib random gets reproducible output.
- **CRC peripheral** added. Standard STM32 CRC-32 unit: DR resets to
  0xFFFFFFFF, default polynomial 0x04C11DB7 (Ethernet). Writes to DR
  step the polynomial engine; CR.RESET reloads DR from INIT.

### Round 7 — RCC PLL state machine + RTC/IWDG/WWDG/DAC (`nucleo_l476rg_pll`, `nucleo_l476rg_misc`)
- **RCC L4 layout** — `RccRegisterLayout::Stm32L4` selector added.
  CFGR moved from offset 0x04 (F1) to 0x08 (L4 has ICSCR at 0x04);
  PLLCFGR added at 0x0C. CR reset value 0x00000063 (MSION+MSIRDY+
  MSIRANGE=6 — boot ROM brings up the 4 MHz MSI before handing off).
- **PLL source-ready gating** — PLLRDY now requires PLLCFGR.PLLSRC's
  selected source to be ready, not just PLLON. HSERDY no longer
  auto-asserts on HSEON unless HSEBYP is also set (NUCLEO can't
  ready HSE without bypass — uses ST-LINK MCO, not a crystal).
- **CFGR.SWS source-lock** — SWS now follows SW only when the requested
  source is ready, matching silicon's "wait for clock to lock" handshake.
- **RTC, IWDG, WWDG, DAC peripherals** added with audited reset values.

### Round 8 — L4 secondary peripherals (`nucleo_l476rg_l4periphs2`)
- **EXTI L4 dual-bank layout** — `ExtiRegisterLayout::Stm32L4` added.
  Bank 1 (lines 0..31) at 0x00..0x14, bank 2 (lines 32..39) at
  0x20..0x34. SWIER1/PR1 latching matches F1 semantics; bank 2 covers
  RTC alarm / USB FS wakeup / LPTIM lines.
- **LPUART1** wired up — register layout is identical to USART (modern
  stm32v2), so reuses the existing UART model.
- **LPTIM1 / LPTIM2** added. ISR/ICR/IER/CFGR/CR/CMP/ARR/CNT/CFGR2/OR.
  ARR/CMP writes set ARROK/CMPOK in ISR (firmware polls these).
  CR.ENABLE clear resets CNT.
- **QUADSPI** added at 0xA0001000. CR/DCR/SR/FCR/DLR/CCR/AR/ABR/DR/
  PSMKR/PSMAR/PIR/LPTR. CCR write with non-zero FMODE asserts SR.TCF
  immediately so survival-mode HAL polling exits.
- **SAI1 / SAI2** added. Two sub-blocks (A/B) sharing the register
  file: GCR + ACR1/ACR2/AFRCR/ASLOTR/AIM/ASR/ACLRFR/ADR + Bx mirror.
- **USB OTG FS** stubbed. Synopsys DWC2 register window @ 0x50000000.
  GUSBCFG=0x1440 (TRDT=0x9, device mode), GRSTCTL.AHBIDL=1 so the
  HAL_PCD core-reset poll exits; sparse write-through for the long
  tail of channel/EP regs.
- **bxCAN1** added. MCR.INRQ -> MSR.INAK handshake, MCR.SLEEP ->
  MSR.SLAK, TSR.TMEx mailbox-empty bits all set. HAL_CAN_Init pattern
  works (set INRQ, poll INAK; configure BTR; clear INRQ, poll INAK
  cleared).
- **Hardware-validation deltas** found and patched:
  - SAI1 ACR1/BCR1 reset is `0x40` (NODIV bit set, "no master clock
    divider"), not 0. Sim default fixed.
  - USB OTG GINTSTS reset is `0x1400_0020` on a NUCLEO with no cable
    plugged: NPTXFE | PTXFE | CIDSCHG | DISCINT. Sim previously had
    `0x0400_0001` (CMOD-style — wrong bits entirely).
  - bxCAN MSR after writing MCR.INRQ=1 is `0x0000_0409` (INAK + WKUI
    + SAMP) — INRQ also latches the WKUI flag, not just INAK. Reset
    value (before INRQ) is `0x0000_040A` (SLAK + SAMP).
  - LPUART1.ISR matches sim's stm32v2 default (`0x000000C0`) when the
    UART is not yet enabled; full reset (`0x00C0_0020` with REACK)
    only manifests post-CR1.UE — outside this firmware's path.

### Round 9 — CubeMX-style HAL bring-up (`nucleo_l476rg_cubemx_hal`)
End-to-end exercise of the canonical STM32CubeIDE-generated firmware
pattern: vector table at `.isr_vector`, `Reset_Handler` doing .data
copy + .bss zero, `SystemInit` setting VTOR + CPACR (FPU enable),
`HAL_Init` programming SysTick at 1 ms with `uwTick++` in
`SysTick_Handler`, `SystemClock_Config` walking PWR voltage scaling +
FLASH 4-WS latency + MSI->PLL@80MHz with the SWS source-lock
handshake, `MX_USART2_UART_Init`, then `HAL_Delay`-paced print loop.

The simulator handles the entire flow without faulting and the
locked trace matches the expected HAL output byte-for-byte. Things
this round actually validates:

- Vector-table relocation via SCB.VTOR (not just ELF entry symbol).
- FPU enable via CPACR (CP10/CP11 = full access).
- PWR.CR1.VOS write + PWR.SR2.VOSF poll handshake.
- FLASH.ACR latency write with read-back loop.
- Full RCC PLL state machine: PLLCFGR programming, CR.PLLON,
  CR.PLLRDY poll, CFGR.SW write, CFGR.SWS source-lock poll.
- SysTick interrupt-driven timekeeping: SYST_CSR enable + TICKINT,
  SHPR3 priority byte, SysTick exception (15) routed via the
  user-supplied vector table to a Rust handler that increments
  `UW_TICK`. `HAL_Delay()` polls the same global to gate progression.
- BRR=694 at 80 MHz → 115200 baud verified via JLink register probe
  on real silicon (`USART2_BRR=0x208D`, `RCC_CFGR=0x0F` confirms
  HCLK=80 MHz).

**Hardware capture caveat**: the J-Link OB Virtual COM Port on this
NUCLEO drops bytes intermittently around USB packet boundaries when
the firmware sends short bursts. Clean fragments seen on
`/dev/ttyACM1` (e.g. `TICK 2 T=4\r\n`, `DONE\r\n`) confirm the wire-
level data matches sim — but a stable byte-for-byte capture for
locking against the survival trace would require a different
debugger or USB-to-UART bridge. The sim baseline is treated as
authoritative until a clean hardware capture is available.

## Reproducing a capture

```bash
# Start cat in background
stty -F /dev/ttyACM1 115200 cs8 -cstopb -parenb -ixon -ixoff -icanon -echo raw
cat /dev/ttyACM1 > capture.bin &
CAT=$!
sleep 0.3

# Flash and reset
JLinkExe -NoGui 1 -AutoConnect 1 -Device STM32L476RG -If SWD -Speed 4000 \
  -CommanderScript flash.jlink
sleep 3

kill $CAT
xxd capture.bin
```

`flash.jlink`:
```
halt
erase
loadfile your-firmware.elf
r
g
qc
```
