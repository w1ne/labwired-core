# Validation — EK-RA8M1 (R7FA8M1AH)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no RA8M1 bench part, and no Renode parity claim is made.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the code-flash vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x2000800, SP: 0x220e0000` |
| Cortex-M85 core executing Armv8-M Mainline firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| Helium/MVE, LOB, PACBTI, M85 caches | **not modelled** | not decoded; the firmware is built for `thumbv8m.main` and never emits them |
| `SCICKCR` CKSREQ/CKSRDY clock-switch handshake | smoke-tested | the firmware spins on CKSRDY both ways and prints `scickcr=01` (MOCO) read back |
| SCICLK rate, `SCKDIVCR`, PLLs, `PRCR` key | **not modelled** | the SCICKCR source is stored only; the other SYSTEM registers read 0 and drop writes |
| `MSTPCRB.MSTPB22` release of SCI9 | declared only | `MSTP` is a stub; SCI9 runs with the bit set too |
| SCI9 (SCI_B) async TX at 9600 8N1 (`CCR2.BRR` 25 from 8 MHz) | smoke-tested | every console line, polled on `CSR.TDRE`; `brr=25` and `ccr3=00001200` read back |
| SCI_B `CCR0.TE` gate and `CESR.TIST/RIST` | smoke-tested | the firmware waits for `cesr=11` before its first `TDR` write; unit test `test_sci_b_tx_gate_status_and_readback` proves TX is dropped while TE is 0 |
| `PWPRS` unlock + `PmnPFS` write (S1/S2 `PCR` pull-up) | smoke-tested | `s1=1 s2=1`: the pads read high through the PFS pull-up link; unit test `ra8_layout_moves_pwpr_and_adds_the_secure_twin` |
| `PmnPFS` `PSEL`/`PMR` (PA14/PA15 → SCI9) | declared only | stored, no routing effect |
| PORT `PCNTR1` PDR/PODR, `PCNTR3` POSR/PORR, `PCNTR2` PIDR on ports 0, 1, 4, 6 | smoke-tested | `led=blue on pin=1 port=6 podr=0001` … (one POSR/PORR store per LED; the whole-PODR read proves no other pin of that port moved) |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| ICU / IELSR event linking, SCI interrupts, SCI_B FIFO | **not modelled** | the ICU is a stub; the smoke is polled |
| TrustZone (IDAU/SAU, CPSCU/PSCU, non-secure aliases) | **not modelled** | the image runs secure, as on silicon out of reset; nothing checks attribution |
| DMAC/DTC, FCACHE, FACI, ELC, RTC, IWDT/WDT, AGT, USBFS, IIC0-2, GPT, ADC12, USBHS, ETHERC/EDMAC, SPI0/1, CANFD | declared stubs | `configs/chips/r7fa8m1ah.yaml` (`type: "stub"`) |
| SCI0-8, OSPI/xSPI, SDRAM, SDHI, SSIE, CEU, DAC, CRC, DOC, RSIP, option-setting memory, standby SRAM | **not mapped** | no window in `configs/chips/r7fa8m1ah.yaml`; an access faults with a memory violation |

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
