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

| Trace                  | Fixture ELF                              | Hardware capture file                                    |
|------------------------|------------------------------------------|----------------------------------------------------------|
| `nucleo_l476rg_smoke`  | `tests/fixtures/nucleo-l476rg-smoke.elf` | `tests/fixtures/hw_traces/nucleo_l476rg_smoke.txt`       |
| `nucleo_l476rg_spi`    | `tests/fixtures/nucleo-l476rg-spi.elf`   | `tests/fixtures/hw_traces/nucleo_l476rg_spi.txt`         |
| `nucleo_l476rg_i2c`    | `tests/fixtures/nucleo-l476rg-i2c.elf`   | `tests/fixtures/hw_traces/nucleo_l476rg_i2c.txt`         |
| `nucleo_l476rg_adc`    | `tests/fixtures/nucleo-l476rg-adc.elf`   | `tests/fixtures/hw_traces/nucleo_l476rg_adc.txt`         |
| `nucleo_l476rg_dma`    | `tests/fixtures/nucleo-l476rg-dma.elf`   | `tests/fixtures/hw_traces/nucleo_l476rg_dma.txt`         |
| `nucleo_l476rg_demo`   | `tests/fixtures/nucleo-l476rg-demo.elf`  | (built from `crates/firmware-l476-demo`, same trace as sim) |

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
