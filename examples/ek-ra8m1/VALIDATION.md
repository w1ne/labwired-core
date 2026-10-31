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
| ICU / IELSR event linking, SCI interrupts, SCI_B FIFO | **not modelled** | both ICU windows (`0x4000_6000`, `0x4000_C000` WUPEN/IELSR) are stubs; the smoke and the Zephyr console are polled |
| TrustZone (IDAU/SAU, CPSCU/PSCU, non-secure aliases) | **not modelled** | the image runs secure, as on silicon out of reset; nothing checks attribution |
| DMAC/DTC, FCACHE, FACI, ELC, RTC, IWDT/WDT, AGT, USBFS, IIC0-2, GPT, ADC12, USBHS, ETHERC/EDMAC, SPI0/1, CANFD | declared stubs | `configs/chips/r7fa8m1ah.yaml` (`type: "stub"`) |
| SCI0-8, OSPI/xSPI, SDRAM, SDHI, SSIE, CEU, DAC, CRC, DOC, RSIP, option-setting memory, standby SRAM | **not mapped** | no window in `configs/chips/r7fa8m1ah.yaml`; an access faults with a memory violation |

## Stock Zephyr (verified 2026-10-31)

```bash
# fixtures are committed; rebuild (byte-identical) with:
crates/firmware-ek-ra8m1-zephyr/build.sh
#   r7fa8m1ah-zephyr-hello.elf  sha256 e9d0146e44ab6f74...
#   r7fa8m1ah-zephyr-sync.elf   sha256 686cf687fbfe5078...

./target/release/labwired test --script examples/ek-ra8m1/zephyr-hello.yaml --no-uart-stdout
#   PASS  1/1 checks · zephyr-hello · 673370 steps
#   *** Booting Zephyr OS build v3.7.0 ***
#   Hello World! ek_ra8m1/r7fa8m1ahecbd

./target/release/labwired test --script examples/ek-ra8m1/zephyr-sync.yaml --no-uart-stdout
#   PASS  1/1 checks · zephyr-sync · 49641151 steps
#   thread_a: Hello World from cpu 0 on ek_ra8m1!
#   thread_b: Hello World from cpu 0 on ek_ra8m1!

# Determinism: two runs of each, identical uart.log and snapshot.json.
```

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
