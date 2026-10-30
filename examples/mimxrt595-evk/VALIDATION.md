# Validation — MIMXRT595-EVK (MIMXRT595S)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no MIMXRT595 bench part, and no Renode parity claim is made.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from the vector table at `0x0800_1000` (PC/SP init) | smoke-tested | run below: `Initial PC: 0x8001800, SP: 0x20500000`; `vtor=08001000` |
| FlexSPI configuration block at `0x0800_0400` | smoke-tested (read back) | `fcb tag=42464346`. The engine does **not** parse the FCB: it boots from `flash.base + reset_vector_offset` (0x1000) |
| Stock Zephyr v3.7.0 `samples/hello_world` (`mimxrt595_evk/mimxrt595s/cm33`) | smoke-tested | `zephyr-hello.yaml`: `*** Booting Zephyr OS build v3.7.0 ***` / `Hello World! mimxrt595_evk/mimxrt595s/cm33`, 149,425 steps; survival gate `test_mimxrt595_evk_zephyr_survival` |
| Stock Zephyr v3.7.0 `samples/synchronization`: `k_busy_wait` on the OS timer count, `k_msleep` woken by the OS timer match IRQ | smoke-tested | `zephyr-sync.yaml`: thread_a → thread_b → thread_a, 10,760,810 steps, 58,767,050 device cycles (1.224 s = 2 × 100 ms busy-wait + 2 × 500 ms sleep at 48 MHz) |
| Secure aliases (IDAU): FlexSPI `0x1800_0000`, SRAM `0x3000_0000`, peripherals `0x5000_0000`, boot ROM `0x1300_0000` | smoke-tested + integration-tested | the Zephyr images run entirely on them; `tests/rt500_secure_map.rs` (shared backing both ways, load into flash through the alias, secure SP/PC reset vector, fetch slice, mirror end). Same backing, no SAU/IDAU security attribution check |
| Boot-ROM OTP API (`OTP_INIT` `0x1300_7FFF`, `OTP_DEINIT` `0x1300_804D`, `OTP_FUSE_READ` `0x1300_805D`) | integration-tested | LabWired clean-room ROM window (`crates/core/roms/rt500/rt500_rom.S`, not an NXP dump); every fuse reads 0 (blank part), so the FRO trims are 0; `rt500_secure_map::otp_api_reads_blank_fuses` calls the entries on the M33 model |
| CLKCTL0/1 hand-shakes: divider `REQFLAG`, `FROCLKSTATUS.CLK_OK`, `LPOSCCTL0.CLKRDY`, SYSPLL0/AUDIOPLL0 `PFDn_CLKRDY`, FRO tuner `FRO_CAPVAL` | smoke-tested + unit-tested | Zephyr `rt5xx_clock_init` runs unpatched; `lpc::rt5xx_ctl::tests`. Every clock settles at once and the FRO is exactly nominal (capture = `EXP_COUNT`); no frequency changes |
| FlexSPI0 controller (`MCR0.SWRESET`, `STS0` idle, `DLLCR`/`STS2` lock, IP commands) | smoke-tested | Zephyr's `flexspi_clock_set_freq` re-clock: the RT1060 `imxrt_flexspi` model on the RT500 base; device = generic JEDEC NOR reporting the EVK's MX25UM51345G ID; octal-DDR (OPI) commands and the RT500-only registers past the LUT are not modelled |
| OS event timer (`OSTIMER`): Gray-coded 42-bit `EVTIMER` at 1 MHz, `MATCH_L/H`, `OSEVENT_CTRL` flag/enable, `OS_EVENT_IRQn` 41 | smoke-tested + unit-tested | the Zephyr system clock in both samples; `lpc::ostimer::tests` (8 tests). Capture, per-core routing, deep-sleep wake and the 42-bit wrap are not modelled |
| FLEXCOMM12 USART (Arduino-header serial, enabled by the Zephyr board) | executed, not checked | same `lpc_flexcomm_usart` model as FLEXCOMM0, IRQ 64; nothing is wired to it |
| Boot ROM boot flow (FCB/image header, ISP, secure boot), CACHE64 | **not modelled** | the simulator enters the vector table at `flash.base + 0x1000` directly; the NOR window is a flat 8 MiB array; CACHE64 is a stub |
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
| CLKCTL/RSTCTL SET/CLR aliases, IOPCTL plain bank, CLKCTL0/1 status bits | unit-tested | `lpc::rt5xx_ctl::tests` (9 tests) |
| Determinism (same inputs → same output) | smoke-tested | two runs each of the smoke, Zephyr hello and Zephyr synchronization: identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: smoke 200,000 instructions, Zephyr hello 150,000, Zephyr synchronization 2,000,000 — 0 unsupported |
| USART receive | unit-tested only | the smoke enables the RX FIFO but injects no bytes |
| USART interrupts, DMA, baud/frame timing, sync/9-bit/LIN modes, error flags | **not modelled** | the smoke is polled; a byte leaves in zero time |
| GPIO pin interrupts (`INTENA`/`INTENB`/`INTPOL`/`INTEDG`/`INTSTAT`), PINT, SECGPIO | **not modelled** | stored only; PINT is a declared stub (Zephyr's GPIO driver runs `PINT_Init`) |
| IOPCTL pulls, drive, slew, open drain | **not modelled** | an undriven input reads 0 whatever the pull |
| SW1/SW2 polarity | **assumed** | active-low (released = 1) as on the other NXP EVKs; the EVK schematic was not checked |
| SYSCTL0/1, PMC, WWDT0, INPUTMUX, CACHE64_CTRL0, PINT, non-secure SCS bank (`0xE002_E000`, the `SCB_NS->CPACR` write in `SystemInit`) | declared stubs | `configs/chips/mimxrt595s.yaml` (`type: "stub"`) |
| SRAM code-bus aliases (`0x0…`, secure `0x1000_0000`), FLEXCOMM1–11/13–16, CTIMER0–4, MRT, UTICK, SCT, DMA0/1, SDIO, USB, ADC, I3C, LCDIF/MIPI/GPU, PowerQuad, TRNG, the OTP controller, HiFi4 DSP | **not mapped** | no window in `configs/chips/mimxrt595s.yaml`; an access faults with a memory violation |
| Cortex-M33 TrustZone (SAU, secure/non-secure state, banked registers) | **not modelled** | the core always runs as if secure; a secure and a non-secure alias reach the same resource |

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
#   2 passed (test_mimxrt595_evk_smoke_survival, test_mimxrt595_evk_zephyr_survival)

# Stock Zephyr v3.7.0 (rebuild: crates/firmware-mimxrt595-evk-zephyr/build.sh;
# two rebuilds are byte-identical: hello 9ae796e8…, synchronization 7625293c…)
./target/release/labwired test --script examples/mimxrt595-evk/zephyr-hello.yaml
#   *** Booting Zephyr OS build v3.7.0 ***
#   Hello World! mimxrt595_evk/mimxrt595s/cm33
#   PASS  1/1 checks · zephyr-hello · 149425 steps
./target/release/labwired test --script examples/mimxrt595-evk/zephyr-sync.yaml
#   thread_a: Hello World from cpu 0 on mimxrt595_evk!
#   thread_b: Hello World from cpu 0 on mimxrt595_evk!
#   thread_a: Hello World from cpu 0 on mimxrt595_evk!
#   PASS  1/1 checks · zephyr-sync · 10760810 steps

# Negative controls (each fails the Zephyr run):
#   no secure mirrors          -> FAIL at step 0 (reset vector rejected)
#   LABWIRED_RT500_ROM=""      -> FAIL (OTP_FUSE_READ_API fetches zeros)
#   CLKCTL as plain rt5xx_ctl  -> FAIL (spins in CLOCK_EnableLpOscClk)
#   FlexSPI0 as a stub         -> FAIL (spins in flash_deinit on STS0)
#   OS timer as a stub         -> hello still passes (no timed wait);
#                                 zephyr-sync FAILs (k_busy_wait never ends)

cargo test -p labwired-core --test rt500_secure_map
#   5 passed

cargo test -p labwired-core --lib lpc::
#   33 passed (flexcomm_usart 9, gpio 7, rt5xx_ctl 9, ostimer 8)

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
- **Zephyr runs secure.** The stock board target is the secure
  `mimxrt595s/cm33` build, so every address it uses is a secure alias. The
  chip maps those as same-backing mirrors (`memory_regions[].alias_of`); the
  engine has no TrustZone, so there is nothing to attribute. A cross-alias
  self-modifying-code case (store through one view, execute the other) is not
  seen by the decode cache; Zephyr relocates and executes through the same
  view.
- **Idle after the banner.** hello_world then idles with the OS timer match
  ~365 days out (`K_FOREVER`). Under `labwired test` idle fast-forward clamps
  each skip to the remaining *step* budget, so a long `max_steps` would spend
  wall time in tiny skips; the Zephyr scripts stop once their assertions pass.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
- The staged `configs/chips/onboarding/imxrt500.yaml` /
  `configs/systems/onboarding/imxrt500.yaml` descriptors (a generic UART
  fixture, flash window on the boot ROM at `0x0300_0000`, no peripherals) are
  kept and marked superseded.
