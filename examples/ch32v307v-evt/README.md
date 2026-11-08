# WCH CH32V307V-EVT (CH32V307VCT6, QingKe V4F)

WCH's evaluation board for the CH32V307VCT6. The MCU is a 144 MHz RISC-V
**RV32IMAFC**: the QingKe V4F core is the CH32V20x's V4 with a
single-precision FPU. The board's on-board WCH-Link carries the debug-printf
console these smokes read. Live availability: [evidence/vendor-status.md](evidence/vendor-status.md).

Demand: this is the WCH (Tier C) vendor bet in
`DEMAND-ORDERED-TARGETS.md`, with CH32V303/305/307 as the "downstream" step
after the CH32V003 and CH32V203. It is not a survey top-5 MCU.

## What runs

| Smoke | Firmware | Checks |
|---|---|---|
| `ch32fun-fpu-selftest-smoke.yaml` | `fpu-selftest/fpu_selftest.c` on ch32fun | 6/6, and all 81 lines match the host FPU oracle |
| `ch32fun-debugprintf-smoke.yaml` | ch32fun `examples_v30x/debugprintfdemo`, unmodified | 5/5 |
| `ch32fun-blink-smoke.yaml` | ch32fun `examples_v30x/blink`, unmodified | 1/1: PA3 edges 144,000,008 cycles apart (1 s at 144 MHz) |

```sh
cargo run -q -p labwired-cli -- test \
  --script examples/ch32v307v-evt/ch32fun-fpu-selftest-smoke.yaml \
  --output-dir /tmp/v307-fpu
examples/ch32v307v-evt/check-fpu-selftest.py /tmp/v307-fpu
```

The FPU self-test runs every RV32F instruction: FLW/FSW and the compressed
C.FLW/C.FSW/C.FLWSP/C.FSWSP, FADD/FSUB/FMUL/FDIV/FSQRT, the four fused
multiply-adds, FSGNJ[N|X], FMIN/FMAX, FEQ/FLT/FLE, FCLASS, FCVT.W[U].S,
FCVT.S.W[U] and FMV. Each one runs on 15 edge-case vectors (overflow,
subnormal results, ties, signed zeros, ∞×0, quiet and signalling NaN,
integer-conversion limits) under RNE, RTZ, RDN and RUP. Every result and its
fflags are hashed. The test also checks a static rounding mode against frm,
compiler-generated float code, and mstatus.FS = Dirty with SD set.

`fpu-selftest/expected.txt` is not produced by LabWired. It is the same C
source built for the x86-64 build host and run on its SSE/FMA unit through
`fenv.h`. x86 detects tininess after rounding, as RISC-V does, so the two
agree flag for flag. `build-ch32fun-examples.sh` regenerates the ELFs and
expected.txt.

Negative control: `evidence/ch32fun-fpu-selftest-smoke/negative-control/` is
the same firmware with `core: qingke-v4b` (the V203's FPU-less core). It
fails 0/6 with a decode error at the first `fsw`.

## The engine change: RV32F

`crates/core/src/cpu/riscv_fp.rs` computes each operation exactly in f64
(TwoSum for sums, the exact f32 product, and an FMA residual for quotients
and square roots). It then rounds to binary32 itself, so all five rounding
modes (including RMM) and all five flags follow the ISA, with tininess
detected after rounding and the canonical NaN. A unit test diffs 2.4 M
add/sub/mul/div/sqrt/fma cases against the host SSE/FMA unit in four
rounding modes. A planted round-half-even bug fails it.

Only the `qingke-v4f` core string turns F on, as the `QingkeV4F` profile.
Every other RISC-V core still treats an FP opcode as a decode error, and
mstatus.FS stays unwritable. The JIT hands FP instructions back to the
interpreter. Both snapshot formats carry f0..f31 and fcsr; a core without F
writes the same bytes as before.

## WCH's official Arduino core (stock sketches)

The same `ch32v307vct6-ch32fun-smoke` profile runs the Arduino matrix's stock
sketches built with WCH's official core,
[openwch/arduino_core_ch32](https://github.com/openwch/arduino_core_ch32),
through the community PlatformIO `ch32v` platform (`genericCH32V307VCT6`,
`-march=rv32imacxw`, the same pinned revisions as the `ch32v203` row). The
sketches are not modified for this board; L8's F1 bxCAN branch now also
covers `CH32V30x`. The row (`ch32v307` in
[`boards.yaml`](../../validation/arduino-matrix/boards.yaml)) adds
`-DLED_BUILTIN=PA3 -DLW_PWM_PIN=PA3`: the CH32V307VCT6 variant leaves
LED_BUILTIN undefined and the EVT's LEDs are unwired, so the row uses PA3,
the pad the ch32fun blink smoke drives, which is also TIM2_CH4.

| Sketch | What it exercises | Result |
| --- | --- | --- |
| L0 / L1 | core boot, `Serial` on USART1 (PA9), `millis()` from the STK interrupt | ✅ ✅ |
| L2 | `digitalWrite(LED_BUILTIN)` on PA3, ≥2 edges | ✅ |
| L3 | `Wire` — on this variant **I2C2** (PB10 SCL / PB11 SDA) — to an INA219 at 0x40: config 0x399F, bus 0x19CA | ✅ |
| L4 | `SPI` on SPI1 (PA5/6/7, CS PA4) to a MAX31855: frame 0x01901600 | ✅ |
| L5 | `analogRead(A0)` on ADC1: two completed conversions (2, 3, as on the V203 row) | ✅ |
| L6 | `analogWrite(PA3, 128)` on TIM2_CH4, ≥4 edges: 4500 high cycles per 144000-cycle (1 kHz) period = 3.125 %, the core's 12-bit default 128/4096 | ✅ |
| L7 | `micros()` | ⏭️ upstream bug, below |
| L8 | bxCAN1 loopback (F1 register branch; the V30x keeps the map and CAN1EN bit 25) | ✅ |

Evidence: [`evidence/arduino-matrix/`](evidence/arduino-matrix/). Reproduce:

```bash
cargo build -p labwired-cli
python3 validation/arduino-matrix/run_matrix.py --boards ch32v307 --labwired target/debug/labwired
```

What the core needed beyond the CH32V203 row's work (XW decoding was
already there for every QingKe profile, V4F included):

- **The factory word at 0x1FFFF70C.** WCH's V30x SPL reads bit 14 in
  `RCC_PLLConfig` and `RCC_GetClocksFreq` to choose the CH32V307/V305
  (`CH32V30x_D8C`) PLL multiplier table. `Serial.begin` reaches it through
  `uart_init`. It is not in the reference manual. The model is a V307, so it
  reads 0x0000_4000, the smallest value with bit 14 set
  ([`pll_cfg_word.yaml`](../../configs/peripherals/ch32v30x/pll_cfg_word.yaml)).
  Without it, L0 faults on the unmapped read at step 966.
- **64 KB of RAM.** WCH's Link.ld puts the stack top at 0x20010000 (the
  256K/64K option-byte split), and ch32fun links 288K/32K. The profile's
  windows are now the union: 288 KB flash, 64 KB RAM.
- **I2C1, I2C2, SPI1, ADC1, TIM1..TIM4 and CAN1.** These are the F103
  register layouts at the ch32v30x.h bases, with ch32v30x.h IRQ numbers
  (F1 + 16 on the PFIC).

Negative controls
([`evidence/arduino-matrix/negative-controls/`](evidence/arduino-matrix/negative-controls/)):
L0 without the factory word fails with a memory violation and no UART. L3
with the INA219 moved to I2C1 gives `LW_L3_FAIL nack`, which shows that Wire
really drives I2C2. L6 watched on PA1 fails the oracle. L8 with the CAN1
clock gate on bit 26 gives `LW_L8_FAIL`.

**L7 is skipped for an upstream core bug, the same one as on the V003 and V203
rows.** `getCurrentMicros()` assumes a down-counter, but `hw_config.c` starts
the STK with `CTLR = 0xF`, which counts up. A `Serial.print` probe on this
profile measured t0=1996 t1=3999 t2=3929 t3=4999 with CTLR=F. The core's
libprintf drops `%lu`, so the sketch's own FAIL line prints empty values.

## Not modeled

- HPE (hardware prologue/epilogue) saves only the integer registers. An ISR
  that uses the FPU must save its own f registers. Nothing here takes an
  interrupt.
- The V30x peripherals beyond the V20x set: USART3..8, TIM8..10, DVP, SDIO,
  FSMC, Ethernet, USBHS, CAN2, DAC, OPA and RNG. Also SPI2/3, ADC2, DMA,
  EXTI and the CAN interrupts. I2C1/I2C2, SPI1, ADC1, TIM1..4 and CAN1 are
  wired for the Arduino row below and exercised only as far as it goes:
  nothing here claims their DMA requests, remapped pads, input capture,
  encoder modes or ADC scan/injected groups.
- HSE, low-power modes, PFIC nesting and priority grouping, and FPU timing
  (every FP instruction retires in one step).
- Physical-board parity: no CH32V307V-EVT was measured.
