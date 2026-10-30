# Validation — MIMXRT595-EVK (MIMXRT595S)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no MIMXRT595 bench part, and no Renode parity claim is made.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from the vector table at `0x0800_1000` (PC/SP init) | smoke-tested | run below: `Initial PC: 0x8001800, SP: 0x20500000`; `vtor=08001000` |
| FlexSPI configuration block at `0x0800_0400` | smoke-tested (read back) | `fcb tag=42464346`. The engine does **not** parse the FCB: it boots from `flash.base + reset_vector_offset` (0x1000) |
| Boot ROM, FlexSPI controller/LUTs, CACHE64, secure boot | **not modelled** | the NOR window is a flat 8 MiB array; the controller and CACHE64 are stubs |
| Cortex-M33 core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| CLKCTL0/1 clock selects and `PSCCTLn` gates via the `_SET` aliases | smoke-tested (storage only) | `clk frodivsel=1 mainclksela=1 pscctl0=00000100 pscctl1=0000000b`. The values have no effect: every block runs ungated and no simulated frequency changes |
| RSTCTL1 reset release via `PRSTCTLn_CLR` | executed, not checked | storage only; every block runs out of reset, and the words reset to 0 rather than the silicon reset values |
| IOPCTL pad function / input buffer | executed, not checked | storage only; the GPIO and FLEXCOMM models ignore it |
| FLEXCOMM0 `PSELID` (USART select, present bits) | smoke-tested | `pselid=000000f1` (USART selected, USART/SPI/I2C/I2S present) |
| FLEXCOMM0 USART async TX, 8N1, `OSR` 16x / `BRG` 26, FIFO | smoke-tested | every console line, through `FIFOWR`, polled on `FIFOSTAT.TXNOTFULL`; `fifosize=16` |
| USART register layout and edge cases (PSELID present bits and lock, registers dark until `PERSEL` = USART, TX gating, RX FIFO 16 deep + `EMPTYRX`, FIFO interrupt status, 7-bit `DATALEN`, snapshot round-trip) | unit-tested | `lpc::flexcomm_usart::tests` (9 tests) |
| GPIO `DIRSET`/`DIRCLR`, `SET`, `CLR`, `NOT`, byte pin `B`, word pin `W`, `MPIN` under `MASK`, `PIN` read-back | smoke-tested | `set red` … `clr all`; the whole-port `pin0` read proves no other pin moved |
| GPIO input with an external level | smoke-tested | `sw1=1 sw2=1`: the released board buttons hold P0_25/P0_10 high |
| GPIO layout and edge cases | unit-tested | `lpc::gpio::tests` (7 tests) |
| CLKCTL/RSTCTL SET/CLR aliases, IOPCTL plain bank | unit-tested | `lpc::rt5xx_ctl::tests` (4 tests) |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| USART receive | unit-tested only | the smoke enables the RX FIFO but injects no bytes |
| USART interrupts, DMA, baud/frame timing, sync/9-bit/LIN modes, error flags | **not modelled** | the smoke is polled; a byte leaves in zero time |
| GPIO pin interrupts (`INTENA`/`INTENB`/`INTPOL`/`INTEDG`/`INTSTAT`), PINT, SECGPIO | **not modelled** | stored only |
| IOPCTL pulls, drive, slew, open drain | **not modelled** | an undriven input reads 0 whatever the pull |
| SW1/SW2 polarity | **assumed** | active-low (released = 1) as on the other NXP EVKs; the EVK schematic was not checked |
| SYSCTL0/1, PMC, WWDT0, INPUTMUX, CACHE64_CTRL0, FlexSPI0 controller | declared stubs | `configs/chips/mimxrt595s.yaml` (`type: "stub"`) |
| SRAM code-bus alias at `0x0`, secure aliases (`0x1…`, `0x3…`, `0x5…`), FLEXCOMM1–16, CTIMER0–4, OS timer, MRT, UTICK, SCT, DMA0/1, SDIO, USB, ADC, I3C, LCDIF/MIPI/GPU, PowerQuad, TRNG, OTP, HiFi4 DSP | **not mapped** | no window in `configs/chips/mimxrt595s.yaml`; an access faults with a memory violation |

## Commands and evidence (verified 2026-10-30)

```bash
# from the repository root
cargo build -p firmware-mimxrt595-evk-smoke --release --target thumbv8m.main-none-eabi
#   Finished `release` profile — no warnings

./examples/mimxrt595-evk/ci/test.sh
#   PASS  3/3 checks · io-smoke · 4000000 steps
#   Audit summary: unsupported_total: 0

./target/release/labwired \
  --firmware tests/fixtures/mimxrt595-evk-smoke.elf \
  --system configs/systems/mimxrt595-evk.yaml --max-steps 400000
#   Initial PC: 0x8001800, SP: 0x20500000
#   MIMXRT595 EVK SMOKE
#   cpu=cortex-m33 uart=flexcomm0 pselid=000000f1 fifosize=16
#   fcb tag=42464346 vtor=08001000
#   ...

# Determinism: two runs, identical UART + snapshot
cmp out/rt595-run1/uart.log out/rt595-run2/uart.log          # clean
cmp out/rt595-run1/snapshot.json out/rt595-run2/snapshot.json  # clean

cargo test -p labwired-core --test firmware_survival mimxrt595
#   1 passed (test_mimxrt595_evk_smoke_survival)

cargo test -p labwired-core --lib lpc::
#   20 passed (flexcomm_usart 9, gpio 7, rt5xx_ctl 4)

python3 scripts/perf/board_perf.py --boards mimxrt595s
#   mimxrt595s  stm32  step 892.0 · batch 1.4 Ir/step
```

## Notes

- **New models.** The RT500 peripherals are LPC-family blocks (FLEXCOMM,
  LPC high-speed GPIO, CLKCTL/RSTCTL/IOPCTL), not the i.MX RT1xxx LPUART/
  GPIO/IOMUXC, so nothing existing fitted. The FLEXCOMM USART model is named
  `lpc_flexcomm_usart`, not `lpc_usart`: the Renode-import profile heuristic
  maps the `lpc_usart` spelling to a 16550 layout, which this block is not.
- **Pin numbering.** GPIO pins are `32 × port + pin` (P0_14 = 14, P1_0 = 32,
  P3_17 = 113).
- **Clocks.** `cpu_hz` is 48 MHz, the FRO DIV clock the smoke selects. On
  silicon `MAINCLKSELA` resets to the 1 MHz LPOSC and the boot ROM may leave
  another source selected; the clock selects have no simulated effect here.
- **Perf.** The perf spin fixture is shared with the STM32 parts, which have
  the same flash/SRAM bases. Its vector table is at `0x0800_0000`, not at the
  `0x0800_1000` this chip resets from, but the CLI `run` path enters through
  the ELF entry point: a probe after 1,000 steps stops at PC `0x08000112`,
  inside the spin loop. So the numbers measure the spin, not a fault loop.
  Identical batch numbers across Cortex-M chips are expected: every board
  runs the same fixture.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
- The staged `configs/chips/onboarding/imxrt500.yaml` /
  `configs/systems/onboarding/imxrt500.yaml` descriptors (a generic UART
  fixture, flash window on the boot ROM at `0x0300_0000`, no peripherals) are
  kept and marked superseded.
