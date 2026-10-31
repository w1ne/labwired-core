# Validation — LPC1114FN28/102 (DIP-28, breadboard)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no LPC1114 bench part. The LPC111x has no Renode platform, so no
Renode parity claim is made either.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from the flash vector table at `0x0000_0000` (PC/SP init) | smoke-tested | `Initial PC: 0xc0, SP: 0x10001000` |
| Boot ROM (ISP, IAP, vector-checksum check, `SYSMEMREMAP`) | **not modelled** | the boot ROM window is not mapped; the engine boots the flash vector table directly |
| Cortex-M0 core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 2,000,000 steps |
| SYSCON clock-source update handshakes (`SYSPLLCLKUEN`, `MAINCLKUEN` 1/0/1, polled) | smoke-tested | the firmware reaches the banner only after both polls succeed |
| SYSCON PLL lock (`PDRUNCFG.SYSPLL_PD` = 0 → `SYSPLLSTAT.LOCK`) | smoke-tested | `lock=1`; negative control below hangs without it |
| `SYSPLLCTRL` / `MAINCLKSEL` / `SYSAHBCLKCTRL` read-back, `DEVICE_ID` | smoke-tested | `clk pllctrl=23 lock=1 mainclksel=3 ahbclkctrl=1185f`, `device_id=1a40902b` |
| Clock frequencies (IRC, PLL M/P, AHB/UART dividers) | **not modelled** | the core runs at the fixed 48 MHz `cpu_hz`; the UART wire timebase is the divisor in engine cycles (exact for `UARTCLKDIV` = `SYSAHBCLKDIV` = 1) |
| UART `LCR.DLAB` switch of `0x00`/`0x04` to `DLL`/`DLM` | smoke-tested + unit-tested | `uart lcr=03 dl=17 …`; a `DLL` write is not transmitted (negative control below) |
| UART `FDR` fractional divider, `TER.TXEN`, `FCR` FIFO reset, `IIR` RDA | smoke-tested (`FDR`, `TER`) + unit-tested | `fdr=f8 ter=80 baud=115089`; `uart::tests::test_lpc11xx_dlab_divisor_fdr_ter_and_iir` |
| UART async TX, 8N1, polled on `LSR.THRE` | smoke-tested | every console line |
| UART interrupts (THRE, RLS, CTI), modem lines, RS-485, auto-baud | **not modelled** | `IIR` reports RDA only; `IER` is stored; RS-485 registers stored only |
| GPIO masked `DATA` window (address bits 13:2 as the pin mask) | smoke-tested + unit-tested | `led=off data=f7f`, `led=on data=fff`; `mask6` leaves PIO0_7 alone |
| GPIO `DIR` and pad level (latch when output, else outside world, else pull-up) | smoke-tested + unit-tested | `data=f7f`: every input pad reads the IOCON reset pull-up |
| GPIO edge/level interrupt (`IS`/`IBE`/`IEV`/`IE`/`RIS`/`MIS`/`IC`) as a level line | smoke-tested + unit-tested | `irq gpio0 edges=3 mis=000`; `lpc11xx_gpio::tests` (10 tests) |
| IOCON pin function / `MODE` pulls / hysteresis / open drain | **not modelled** | IOCON is stored only; the `FUNC` writes are executed but not checked |
| Start-logic wake-up interrupts (`STARTERP0`, IRQ 0–12), deep sleep, PMU | **not modelled** | stored only / declared stub |
| I2C, WWDT, CT16B0/1, CT32B0/1, ADC, PMU, flash controller, SSP0 | declared stubs | `configs/chips/lpc1114fn28.yaml` (`type: "stub"`) |
| SSP1, C_CAN | **not mapped** | the LPC1114FN28 has neither |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| Firmware build reproducibility | checked | two clean rebuilds, identical ELF (sha256 prefix `2b8636a64792164d`) |

## Commands and evidence (verified 2026-10-31)

```bash
# from the repository root
cargo build -p firmware-lpc1114fn28-smoke --release --target thumbv6m-none-eabi

./examples/lpc1114fn28-dip/ci/test.sh
#   PASS  3/3 checks · io-smoke · 2000000 steps
#   Audit summary: unsupported_total: 0

./target/release/labwired \
  --firmware target/thumbv6m-none-eabi/release/firmware-lpc1114fn28-smoke \
  --system configs/systems/lpc1114fn28-dip.yaml --max-steps 300000
#   Initial PC: 0xc0, SP: 0x10001000
#   LPC1114FN28 SMOKE
#   cpu=cortex-m0 uart=uart pins=pio1_6/pio1_7 led=pio0_7
#   device_id=1a40902b
#   ...

# Determinism: two runs, identical UART + snapshot
cmp out/lpc1114-run1/uart.log out/lpc1114-run2/uart.log          # clean
cmp out/lpc1114-run1/snapshot.json out/lpc1114-run2/snapshot.json  # clean

cargo test -p labwired-core --test firmware_survival lpc1114fn28
#   1 passed (test_lpc1114fn28_dip_smoke_survival)

cargo test -p labwired-core --lib lpc11xx
#   11 passed (10 lpc11xx_gpio + 1 uart)

python3 scripts/perf/board_perf.py --boards lpc1114fn28
#   lpc1114fn28  apollo4p  step 908.5 · batch 1.4 Ir/step
```

Negative controls (each run against a temporary copy of the chip descriptor,
then deleted):

| Change | Result |
| :--- | :--- |
| UART on `profile: "dw_apb_uart"` (16550 map, no `DLAB`) | **FAIL** 2/3: the `DLL` write goes out as a stray `\021` byte and the divisor read-back is wrong |
| SYSCON without the PLL-lock timing hooks | **FAIL** 1/3: the firmware never leaves the `SYSPLLSTAT.LOCK` wait; the UART is silent |

## Notes

- **New models.** `lpc11xx_gpio` is new: it is the ARM PL061 lineage like
  `cc2538_pl061_gpio`, but with a 12-bit mask, the control block at `0x8000`
  and none of the CC2538's AFSEL/LOCK/PMUX registers. The UART is a new
  `lpc11xx` layout in the shared UART model: `DwApbUart` has the same offsets
  but no `DLAB`, so a driver's `DLL` write was transmitted.
- **Port names.** The ports are `pio0`–`pio3`. Named `gpio0`–`gpio3`, the
  bus took them for Nordic ports and ran its GPIOTE edge-service pass every
  cycle (perf-spin 1356.5 Ir/step instead of 908.5).
- **Perf.** The perf spin fixture is shared with the Apollo4 Plus, which has
  the same flash/SRAM bases (`0x0` / `0x1000_0000`) and is built for
  `thumbv6m`, so it runs on the Cortex-M0 unchanged.
- **Fixture checksum.** The committed ELF has vector entry 7 at zero; real
  ISP tools write the LPC vector checksum there when they program the part.
  The engine does not check it.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
