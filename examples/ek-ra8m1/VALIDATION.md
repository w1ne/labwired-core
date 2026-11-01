# Validation — EK-RA8M1 (R7FA8M1AH)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no RA8M1 bench part, and no Renode parity claim is made.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the code-flash vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x2000800, SP: 0x220e0000` |
| Cortex-M85 core executing Armv8-M Mainline firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| Armv8.1-M `DLS`/`WLS`/`LE` loops and `CSEL`/`CSINC`/`CSINV`/`CSNEG` | smoke-tested | the stock Zephyr image runs a `DLS`/`LE` loop in `z_arm_interrupt_init` and `CSINC` in `cbprintf`; unit tests `armv81m_*` run GNU-as-encoded loops and every CSEL variant |
| Helium/MVE, tail-predicated loops, PACBTI, `BF*`, M85 caches | **not modelled** | not decoded; the smoke is built for `thumbv8m.main` and Zephyr for `cortex-m85+nomve`, so neither emits them |
| Stock Zephyr v3.7.0 `hello_world` (`ek_ra8m1`) | smoke-tested | `zephyr-hello.yaml` PASS 1/1 in 673,370 steps; survival gate `test_ek_ra8m1_zephyr_survival`; audit 150,000 steps, 0 unsupported |
| Stock Zephyr v3.7.0 `synchronization`: SysTick timeout wake, semaphore hand-off, PendSV switch | smoke-tested | `zephyr-sync.yaml` PASS 1/1: `thread_a` then `thread_b` after `k_busy_wait(100 ms)` (48 M cycles at Zephyr's 480 MHz) and `k_msleep(500)`, 49,641,151 steps |
| FSP `bsp_clock_init`: MOSC start, PLL1/PLL2 start, ICLK to PLL1P, SCICKCR | smoke-tested | Zephyr's console comes up only after `OSCSF.MOSCSF`/`PLLSF` and `SCICKCR.CKSRDY`; unit tests `ra8_oscsf_follows_the_oscillator_stop_bits`, `ra8_every_peripheral_clock_switch_has_the_handshake` |
| `SCICKCR` CKSREQ/CKSRDY clock-switch handshake | smoke-tested | the firmware spins on CKSRDY both ways and prints `scickcr=01` (MOCO) read back |
| SCICLK rate, `SCKDIVCR`/PLL rates, oscillator settle time, `PRCR` key | **not modelled** | every `R_SYSTEM` register is stored and reads back, but no rate is derived and `cpu_hz` stays 8 MHz; oscillators settle at once; PRCR does not gate writes |
| `MSTPCRB.MSTPB22` release of SCI9 | declared only | `MSTP` is a stub; SCI9 runs with the bit set too |
| SCI9 (SCI_B) async TX at 9600 8N1 (`CCR2.BRR` 25 from 8 MHz) | smoke-tested | every console line, polled on `CSR.TDRE`; `brr=25` and `ccr3=00001200` read back |
| SCI_B `CCR0.TE` gate and `CESR.TIST/RIST` | smoke-tested | the firmware waits for `cesr=11` before its first `TDR` write; unit test `test_sci_b_tx_gate_status_and_readback` proves TX is dropped while TE is 0 |
| `PWPRS` unlock + `PmnPFS` write (S1/S2 `PCR` pull-up) | smoke-tested | `s1=1 s2=1`: the pads read high through the PFS pull-up link; unit test `ra8_layout_moves_pwpr_and_adds_the_secure_twin` |
| `PmnPFS` `PSEL`/`PMR` (PA14/PA15 → SCI9) | declared only | stored, no routing effect |
| PORT `PCNTR1` PDR/PODR, `PCNTR3` POSR/PORR, `PCNTR2` PIDR on ports 0, 1, 4, 6 | smoke-tested | `led=blue on pin=1 port=6 podr=0001` … (one POSR/PORR store per LED; the whole-PODR read proves no other pin of that port moved) |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| Stock Zephyr v4.0.0 `samples/drivers/counter/alarm` (AGT0 compare match A through the ICU) | smoke-tested | `zephyr-counter-alarm.yaml` PASS 1/1: alarms at 2/6/14/30 s of guest time, then the 32 s (65536-tick) request refused; 1,380,716 steps, 14.4 G cycles mostly idle-skipped |
| ICU `IELSRn` event link (`ra_icu`): event sets `IR`, line n follows `IR` as a level, `IR` write-0-to-clear | smoke-tested | the alarm above; `crates/core/tests/ra8m1_agt_icu.rs` (vectors through IELSR84, unlinked event pends nothing, clearing `IR` drops the pend); unit tests `peripherals::ra_icu` |
| AGT0/AGT1 timer mode (`ra_agt`): LOCO/PCLKB count sources, `AGTMR2.CKS`, reload, underflow `TUNDF`, compare A/B, `TSTOP` | smoke-tested | the alarm above; unit tests `peripherals::ra_agt` |
| Stock Zephyr v3.7.0 `tests/drivers/uart/uart_basic_api` (interrupt-driven SCI_B driver, RXI/TXI/TEI through the ICU) | smoke-tested | `zephyr-uart-basic-api.yaml` PASS 2/2: `uart_basic_api` 6/6 (`fifo_fill` on TXI/TEI, `fifo_read` on RXI with the driver's default receive FIFO, `poll_in`, `poll_out`) and `uart_basic_api_pending` 1/1; 35,125,173 steps, 2.8 s |
| SCI_B RXI/TXI/TEI as ELC events (`elc_event` on sci0/sci2/sci3/sci9): TXI on TDRE rising or TIE set with TDRE = 1, TEI on TEND rising or TEIE set with TEND = 1, RXI once per character a frame apart; frames of 10 bits at `baud` in `clock_hz` cycles | smoke-tested | the test above; `crates/core/tests/ra8m1_sci_icu.rs` (TXI vectors through IELSR25, TEI one frame after the last byte, RXI per character a frame apart, no RXI without RIE) in both scheduler modes; unit tests `peripherals::uart::sci_b_irq` |
| SCI_B `FRSR.R` receive-FIFO count | smoke-tested | Zephyr's `fifo_read` reads it (`CONFIG_UART_RA_SCI_B_UART_FIFO_ENABLE=y` is the default); integration test above |
| SCI_B FIFO trigger levels (`FCR.RTRG`, `FRSR.DR` 15-ETU timeout), ERI and the error flags, `CCR2` baud decode, TX FIFO occupancy | **not modelled** | RXI is raised per character instead of at RTRG; `FRSR.DR` and `FTSR` read 0 (a written byte leaves at once); the frame time comes from YAML `baud`, not `CCR2` |
| Stock Zephyr v4.1.0 `samples/basic/button` (S1 on P009 → IRQ13 → ELC event 0x00E → ICU IELSR89 → NVIC line 89) | smoke-tested | `zephyr-button.yaml` PASS 5/5: two presses give exactly two `Button pressed at` lines (cycles 498,960,451 and 1,498,992,451), the releases none, and LED1 (P600) mirrors S1 (4 edges, ends low); 5,369,263 steps; audit 2,000,000 steps, 0 unsupported |
| ICU IRQ-pin half (`ra_icu` at `0x4000_6000`, `ielsr_lines: 0`): `IRQCRn.IRQMD` falling/rising/both/low edge, `PmnPFS.ISEL` gate, `ICU_IRQn` = event n + 1 linked by `icu_ielsr`; pad → IRQn table from Zephyr `r7fa8m1xh.dtsi` | smoke-tested | the button above; unit tests `peripherals::ra_icu` (`irqmd_selects_the_edge_that_raises_icu_irqn`, `a_pad_without_isel_raises_nothing`, `the_irq_pin_half_alone_is_not_an_event_link_controller`) |
| ICU NMI, `IRQCRn` digital filter (`FLTEN`/`FCLKSEL`), low-level re-raise while the pad stays low, `DTCE`, re-entry while `IR` stays set | **not modelled** | NMI registers stored only; an edge is taken at once; low-level mode raises on the falling edge only |
| `PmnPFS` `PDR`/`PODR` → `PCNTR1` (the same flip-flops on silicon) | sim-tested | Stock Zephyr v4.1.0 `samples/basic/blinky` (`zephyr-blinky.yaml` PASS 4/4): P600 made an output by its PmnPFS store and toggled five times through the 16-bit `PCNTR1.PODR` alias, exactly five `LED state` lines ending OFF, 6 edges on `port6:0`, `PCNTR1` ends PDR set / PODR clear; 1,304,065 steps; audit 1,000,000 steps, 0 unsupported; fixture sha256 `671464cdd92a9b4b…`. The button sample's LED1 mirror of S1 now shows 4 edges and ends low (5,369,263 steps). Without the PFS → port hand-over both scripts fail. Unit test `pins::pad_control::tests::ra_pfs_low_byte_stores_write_the_port_output`, `peripherals::gpio::tests::ra_port_16_bit_aliases_follow_the_part`; integration `pad_control_pulls::ra_pfs_pdr_podr_are_the_port_output_flip_flops` |
| `PmnPFS` read-back of PDR/PODR after `PCNTR1`/`PCNTR3` writes; `PmnPFS.PIDR` | **not modelled** | PmnPFS returns the last store; PIDR there reads 0 |
| AGT pulse output, event counter, pulse-width/period measurement, AGT1 counting AGT0 underflow | **not modelled** | `AGTIOC`/`AGTISR`/`AGTIOSEL` stored only |
| TrustZone (IDAU/SAU, CPSCU/PSCU, non-secure aliases) | **not modelled** | the image runs secure, as on silicon out of reset; nothing checks attribution |
| DMAC/DTC, FCACHE, FACI, ELC, RTC, IWDT/WDT, USBFS, IIC0-2, GPT, ADC12, USBHS, ETHERC/EDMAC, SPI0/1, CANFD | declared stubs | `configs/chips/r7fa8m1ah.yaml` (`type: "stub"`) |
| SCI1, SCI4-8, OSPI/xSPI, SDRAM, SDHI, SSIE, CEU, DAC, CRC, DOC, RSIP, option-setting memory, standby SRAM | **not mapped** | no window in `configs/chips/r7fa8m1ah.yaml`; an access faults with a memory violation |

## Stock Zephyr (verified 2026-10-31, button and blinky 2026-11-01)

```bash
# fixtures are committed; rebuild (byte-identical) with:
crates/firmware-ek-ra8m1-zephyr/build.sh
#   r7fa8m1ah-zephyr-hello.elf  sha256 e9d0146e44ab6f74...
#   r7fa8m1ah-zephyr-sync.elf   sha256 686cf687fbfe5078...
#   r7fa8m1ah-zephyr-counter-alarm.elf  sha256 e4bd93a4991e21ca... (Zephyr v4.0.0)
#   r7fa8m1ah-zephyr-uart-basic-api.elf  sha256 9c0328041125498b...
#   r7fa8m1ah-zephyr-button.elf  sha256 ffbfe48e747d292e... (Zephyr v4.1.0)
#   r7fa8m1ah-zephyr-blinky.elf  sha256 671464cdd92a9b4b... (Zephyr v4.1.0)

./target/release/labwired test --script examples/ek-ra8m1/zephyr-hello.yaml --no-uart-stdout
#   PASS  1/1 checks · zephyr-hello · 673370 steps
#   *** Booting Zephyr OS build v3.7.0 ***
#   Hello World! ek_ra8m1/r7fa8m1ahecbd

./target/release/labwired test --script examples/ek-ra8m1/zephyr-sync.yaml --no-uart-stdout
#   PASS  1/1 checks · zephyr-sync · 49641151 steps
#   thread_a: Hello World from cpu 0 on ek_ra8m1!
#   thread_b: Hello World from cpu 0 on ek_ra8m1!

./target/release/labwired test --script examples/ek-ra8m1/zephyr-counter-alarm.yaml --no-uart-stdout
#   PASS  1/1 checks · zephyr-counter-alarm · 1380716 steps
#   Set alarm in 2 sec (4096 ticks) / !!! Alarm !!! / Now: 2
#   ... Now: 6 / Now: 14 / Now: 30
#   Set alarm in 32 sec (65536 ticks) / Alarm could not be set

./target/release/labwired test --script examples/ek-ra8m1/zephyr-uart-basic-api.yaml --no-uart-stdout
#   PASS  2/2 checks · zephyr-uart-basic-api · 35125173 steps
#   SUITE PASS - 100.00% [uart_basic_api]: pass = 6, fail = 0, skip = 0, total = 6
#   SUITE PASS - 100.00% [uart_basic_api_pending]: pass = 1, fail = 0, skip = 0, total = 1

./target/release/labwired test --script examples/ek-ra8m1/zephyr-button.yaml --no-uart-stdout
#   PASS  5/5 checks · zephyr-button · 5369263 steps
#   *** Booting Zephyr OS build v4.1.0 ***
#   Set up button at gpio@40400000 pin 9 / Set up LED at gpio@404000c0 pin 0
#   Press the button
#   Button pressed at 498960451
#   Button pressed at 1498992451
#   LED1 (port6:0): 4 edges; PCNTR1 ends PDR0 set, PODR0 clear

./target/release/labwired test --script examples/ek-ra8m1/zephyr-blinky.yaml --no-uart-stdout
#   PASS  4/4 checks · zephyr-blinky · 1304065 steps
#   LED state: OFF / ON / OFF / ON / OFF; 6 edges on port6:0
#   Audit (1,000,000 instructions): unsupported_total: 0

# Determinism: two runs of each, identical uart.log and snapshot.json.
```

Negative controls for `zephyr-uart-basic-api.yaml` (each one fails it):
without `elc_event` on sci9 (the polled SCI_B of the earlier slices),
`test_uart_fifo_fill` fails its assertion and `test_uart_fifo_read` never
gets a character; without `FRSR.R`, the driver's FIFO read takes nothing and
prints a NUL; without the one-frame RXI spacing, the second character's RXI
lands while the first handler runs, before it clears `IELSR24.IR`, and is
lost with it (`fifo_read` stops after "T").

The v4.0.0 `ek_ra8m1` board enables SCI3 (the mikroBUS UART, `&uart3`) as
well as SCI9, and opens it at boot: without the `sci3` window the image
BusFaults at `0x4035_8308` in `R_SCI_B_UART_Open`.

The v4.1.0 `ek_ra8m1` board also opens SCI0 (PMOD1) and SCI2 (PMOD2) at
boot: without the `sci0` window the image BusFaults at `0x4035_8008` in
`R_SCI_B_UART_Open`.

Negative control for `zephyr-blinky.yaml` and the LED half of
`zephyr-button.yaml`: with the port ignoring PmnPFS PDR/PODR stores (as
before), P600 stays an input, so blinky has no edges and `PCNTR1` reads 0,
and the button LED shows none; both fail.

Negative control for `zephyr-button.yaml`: the `icu` window without its
`irq_pins` (the IRQ-pin half inert, as the old stub was) prints "Press the
button" and then nothing, and the regex fails.

Negative controls for `zephyr-counter-alarm.yaml` (each one fails it): AGT0
as a stub (the console stops after "Counter alarm sample"), and the
`icu_ielsr` window as a stub (the compare matches but no alarm line is
printed, because no IELSR links the event to line 84).

Negative controls (each one fails `zephyr-hello.yaml`):

- the old `ra_sysc` model (HOCO + SCICKCR only): FSP spins on
  `OSCSF.MOSCSF` in `bsp_clock_init` and the console never comes up;
- without the `icu_ielsr` window: Precise BusFault at `0x4000_C360` in
  `uart_ra_sci_b_init_0`;
- without `DLS`/`LE` decode, the unit test
  `armv81m_dls_le_loop_runs_the_body_lr_times` documents the old behaviour:
  the legacy fallback skipped both instructions and the loop body ran once.

The Zephyr ELF also carries the option-setting memory (`0x0300_A100`+:
OFS0/OFS1/SECMPU). That region is not mapped; the loader drops those
segments with a warning, and the reset defaults apply.

## Commands and evidence (verified 2026-10-30)

```bash
# from the repository root
cargo build -p firmware-ek-ra8m1-smoke --release --target thumbv8m.main-none-eabi
#   Finished `release` profile — no warnings

./examples/ek-ra8m1/ci/test.sh
#   PASS  3/3 checks · io-smoke · 4000000 steps
#   Audit summary: unsupported_total: 0

./target/release/labwired \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-ek-ra8m1-smoke \
  --system configs/systems/ek-ra8m1.yaml --max-steps 600000
#   Initial PC: 0x2000800, SP: 0x220e0000
#   EK-RA8M1 SMOKE
#   cpu=cortex-m85 uart=sci9 (sci_b)
#   clk src=moco sciclk=8000000 scickcr=01 brr=25 baud=9600
#   ccr3=00001200 cesr=11
#   led=blue on pin=1 port=6 podr=0001
#   ...
#   s1=1 s2=1
#   hb=0 podr6=0001 s1=1 s2=1

# Determinism: two runs, identical UART + snapshot
cmp out/ek-ra8m1-run1/uart.log out/ek-ra8m1-run2/uart.log          # clean
cmp out/ek-ra8m1-run1/snapshot.json out/ek-ra8m1-run2/snapshot.json  # clean

cargo test -p labwired-core --test firmware_survival ek_ra8m1
#   1 passed (test_ek_ra8m1_smoke_survival)
cargo test -p labwired-core --lib -- sci_b ra_pfs ra_clock
#   9 passed (incl. the new SCI_B, RA8 PFS and RA8 SYSTEM tests)
```

## Notes

- **New models.** The RA8 SCI is SCI_B, a different register block from the
  RA4M1/RA6M5 SCI, so the UART gains a `sci_b` layout (`profile: "sci_b"`;
  the `renesasra8m1_sci` type the Renode importer emits also maps to it). The
  RA8 PMISC moved `PWPR` to +0x0C and added the secure `PWPRS` at +0x14
  (`ra8_pfs`), and SCI runs from its own SCICLK behind a CKSREQ/CKSRDY
  handshake (`ra8_sysc`). The bases and fields are from `R7FA8M1AH.h`.
- **Reset values.** The SCI_B `CCR2` (0xFF00FF04) and `FCR` (0x1F1F0000)
  reset values are the FSP driver's `*_DEFAULT_VALUE` constants; the hardware
  manual was not consulted. `SCICKCR`/`SCICKDIVCR` reset to 0 in the model.
- **Vector table size.** `cortex-m-rt` reserves 496 external vectors for
  ARMv8-M without a device crate, so `.text` starts at `0x0200_0800`.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
- The staged `configs/chips/onboarding/renesas-ek_ra8m1.yaml` puts flash at
  `0x08000000` and RAM at `0x30000000`, both placeholders. It is kept, marked
  superseded.
