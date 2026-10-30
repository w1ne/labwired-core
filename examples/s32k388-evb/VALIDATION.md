# Validation — S32K388EVB-Q289 (S32K388)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no S32K388 bench part, and no Renode parity claim is made.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot of CM7_0 from the vector table at `0x0040_1000` (PC/SP init) | smoke-tested | run below: `Initial PC: 0x401400, SP: 0x204c0000`; `vtor=00401000` |
| Boot header (IVT) at `0x0040_0000` | smoke-tested (read back) | `ivt marker=5aa55aa5 cm7_0=00401000`. The engine does **not** parse the IVT: it boots from `flash.base + reset_vector_offset` (0x1000), so an image whose IVT names another CM7_0 address will not boot correctly |
| Boot ROM (SBAF), HSE firmware, secure boot | **not modelled** | |
| Cortex-M7 core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| `SWT_0` unlock (`SR` keys) + `CR.WEN` clear | executed, not checked | the SWT_0 window is a stub; the watchdog never fires, with or without the writes |
| MC_ME partition-1 clock gating (`COFB1/2_CLKEN`, `PCONF`, `PUPD`, `CTL_KEY`) | executed, not checked | the MC_ME window is a stub; gating has no effect and every block runs ungated |
| Clock tree (FIRC/FXOSC/PLL, MC_CGM muxes and dividers) | **not modelled** | the core stays at the 48 MHz FIRC reset clock; LPUART frame timing uses the configured 24 MHz `AIPS_SLOW_CLK` |
| LPUART2 async TX, 8N1, `BAUD` OSR 16 / SBR 13 | smoke-tested | every console line, polled on `STAT.TDRE` |
| SIUL2 `MSCR` (`SSS` = GPIO, `OBE`, `IBE`, `PUE`/`PUS`) | smoke-tested | LED pads drive only as GPIO outputs; `pta1 pu=1 pd=0` |
| SIUL2 `GPDO` byte writes + `GPDI` byte reads (byte-swapped within each word) | smoke-tested | `on ptf17 gpdi=1 …` for all six pads |
| SIUL2 `PGPDO11`/`PGPDI11` (bit-reversed, halfword-swapped) | smoke-tested | `pgpdo11=4000` … `7380`; the whole-port read proves no other pin moved |
| SIUL2 `MPGPDO11` masked write | smoke-tested | `mpgpdo pgpdo11=0380` (only PTF17–19 cleared). The mask/data bit order follows PGPDO by analogy and is not confirmed on silicon |
| SIUL2 register layout and edge cases (reserved MSCRs, `DISR0` W1C, read-only `MIDR`, node reset) | unit-tested | `s32k3_siul2::tests` (7 tests) |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| SIUL2 input mux (`IMCR`), `INV`, open drain, drive/slew | **not modelled** | `IMCR` is stored only |
| SIUL2 external interrupts / filters (`DISR0`, `DIRER0`, …) | **not modelled** | stored only; `DISR0` never sets |
| LPUART2 pad mux | **not exercised** | the smoke does not mux LPUART2 onto pads; the console pins are not confirmed (see `REQUIRED_DOCS.md`) |
| LPUART/SIUL2 interrupts | **not modelled** in this slice | the smoke is polled |
| SRAM/TCM ECC | **not modelled** | an uninitialised SRAM read returns 0 instead of raising an ECC error |
| Caches, TCM use, MPU, FPU | **not exercised** | the firmware leaves them at reset and is built without FPU instructions |
| CM7_1, CM7_2, CM7_3 and the lockstep checkers | **not modelled** | only CM7_0 runs |
| MSCM, PFLASH, SWT_0, STM_0, XRDC, DMAMUX0/1, MC_RGM, DCM_GPR, SIRC, SXOSC, FIRC, FXOSC, MC_CGM, MC_ME, PLL, PLL_AUX, PMC, flash controller, CONFIGURATION_GPR | declared stubs | `configs/chips/s32k388.yaml` (`type: "stub"`) |
| Data flash, eDMA/TCDs, SWT_1–3, STM_1–3, PIT, RTC, FlexCAN, LPSPI, LPI2C, eMIOS, ADC/BCTU, GMAC, QuadSPI, SAI, FlexIO, HSE/MU, CRC, WKPU | **not mapped** | no window in `configs/chips/s32k388.yaml`; an access faults with a memory violation |

## Commands and evidence (verified 2026-10-30)

```bash
# from the repository root
cargo build -p firmware-s32k388-evb-smoke --release --target thumbv7em-none-eabi
#   Finished `release` profile — no warnings

./examples/s32k388-evb/ci/test.sh
#   PASS  3/3 checks · io-smoke · 4000000 steps
#   Audit summary: unsupported_total: 0

./target/release/labwired \
  --firmware target/thumbv7em-none-eabi/release/firmware-s32k388-evb-smoke \
  --system configs/systems/s32k388-evb.yaml --max-steps 400000
#   Initial PC: 0x401400, SP: 0x204c0000
#   S32K388 EVB SMOKE
#   cpu=cortex-m7 core=cm7_0 uart=lpuart2
#   ivt marker=5aa55aa5 cm7_0=00401000 vtor=00401000
#   ...

# Determinism: two runs, identical UART + snapshot
cmp out/s32k388-run1/uart.log out/s32k388-run2/uart.log          # clean
cmp out/s32k388-run1/snapshot.json out/s32k388-run2/snapshot.json  # clean

cargo test -p labwired-core --test firmware_survival s32k388
#   1 passed (test_s32k388_evb_smoke_survival)

cargo test -p labwired-core --lib s32k3_siul2
#   7 passed

python3 scripts/perf/board_perf.py --boards s32k388
#   s32k388  same70  step 907.8 · batch 1.4 Ir/step
```

## Notes

- **New model.** `s32k3_siul2` is new: the S32K3 SIUL2 is not the Kinetis
  PORT/GPIO pair, so nothing existing fitted. LPUART0–15 reuse
  `imxrt_lpuart`, which has the same register layout (`VERID`/`PARAM`/
  `GLOBAL`/`PINCFG` in front of `BAUD` at `0x10`).
- **Pin numbering.** Pins are the `MSCR` index, `32 × port + pin`
  (PTA1 = 1, PTF23 = 183). The Renode board file puts SW11 (PTB8) at index
  28, which is not `32 × 1 + 8`; SW11 is not used here.
- **Perf.** The perf spin fixture is shared with the SAM E70, which has the
  same flash/SRAM bases. Its vector table is at `0x0040_0000`, not at the
  `0x0040_1000` this chip resets from, but the CLI `run` path enters through
  the ELF entry point: a probe after 1,000 steps stops at PC `0x00400112`,
  inside the spin loop. So the numbers measure the spin, not a fault loop.
  Identical batch numbers across Cortex-M chips are expected: every board
  runs the same fixture.
- **Toolchain.** The firmware is built for `thumbv7em-none-eabi`
  (soft-float), so the compiler emits no FPU instructions.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
- The staged `configs/chips/onboarding/nxp-s32k388.yaml` /
  `nxp-s32k388evb.yaml` descriptors (a generic UART fixture, 256 KiB of RAM,
  no SIUL2 model) are kept and marked superseded.
