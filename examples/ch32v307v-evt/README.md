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

## Not modeled

- HPE (hardware prologue/epilogue) saves only the integer registers. An ISR
  that uses the FPU must save its own f registers. Nothing here takes an
  interrupt.
- The V30x peripherals beyond the V20x set: USART3..8, TIM8..10, DVP, SDIO,
  FSMC, Ethernet, USBHS, CAN2, DAC, OPA and RNG. Also I2C, SPI, timers, ADC,
  DMA and EXTI, which are not wired on this profile yet.
- HSE, low-power modes, PFIC nesting and priority grouping, and FPU timing
  (every FP instruction retires in one step).
- Physical-board parity: no CH32V307V-EVT was measured.
