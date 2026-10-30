# Validation — EK-RA6M5 (R7FA6M5BH)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no RA6M5 bench part, and no Renode parity claim is made.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the code-flash vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x800, SP: 0x20080000` |
| Cortex-M33 core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| `PRCR`-guarded `SCKDIVCR` write (/1 on the reset MOCO) | declared only | the SYSTEM window drops the write; the baud math in the firmware assumes the silicon result (`clk src=moco pclka=8000000 brr=25 baud=9600`) |
| Clock generation (HOCO/PLL/dividers) | **not modelled** | `ra_sysc` models only `HOCOCR`/`OSCSF`; the PRCR key is not checked |
| `MSTPCRB.MSTPB31` release of SCI0 | declared only | `MSTP` is a stub; SCI0 runs with the bit set too |
| SCI0 async TX at 9600 8N1 (`BRR 25` from 8 MHz) | smoke-tested | every console line, polled on `SSR.TDRE` |
| `PWPR` unlock + `PmnPFS` write (S1/S2 `PCR` pull-up) | smoke-tested | `s1=1 s2=1`: the pads read high through the PFS pull-up link |
| `PmnPFS` `PSEL`/`PMR` (P410/P411 → SCI0) | declared only | stored, no routing effect |
| PORT0 `PCNTR1` PDR/PODR, `PCNTR3` POSR/PORR, `PCNTR2` PIDR | smoke-tested | `led=blue on pin=1 podr=0040` … (one POSR/PORR store per LED; the whole-PODR read proves no other pin moved) |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| ICU / IELSR event linking, SCI interrupts | **not modelled** | the ICU is a stub; the smoke is polled |
| TrustZone (IDAU/SAU, CPSCU/PSCU) | **not modelled** | the image runs secure, as on silicon out of reset; nothing checks attribution |
| DMAC/DTC, ELC, RTC, IWDT/WDT, USBFS, IIC0-2, CANFD, AGT0-5, USBHS, ETHERC/EDMAC, SPI0/1, GPT, ADC12, FACI | declared stubs | `configs/chips/r7fa6m5bh.yaml` (`type: "stub"`) |
| SCI1-9, QSPI/OSPI, SDHI, SSIE, CTSU, CEC, DAC12, CRC, DOC, data flash, option-setting memory | **not mapped** | no window in `configs/chips/r7fa6m5bh.yaml`; an access faults with a memory violation |

## Commands and evidence (verified 2026-10-30)

```bash
# from the repository root
cargo build -p firmware-ek-ra6m5-smoke --release --target thumbv8m.main-none-eabi
#   Finished `release` profile — no warnings

./examples/ek-ra6m5/ci/test.sh
#   PASS  3/3 checks · io-smoke · 4000000 steps
#   Audit summary: unknown_thumb16: 0 · unhandled_thumb32: 0 · unsupported_total: 0

./target/release/labwired \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-ek-ra6m5-smoke \
  --system configs/systems/ek-ra6m5.yaml --max-steps 400000
#   Initial PC: 0x800, SP: 0x20080000
#   EK-RA6M5 SMOKE
#   cpu=cortex-m33 uart=sci0
#   clk src=moco pclka=8000000 brr=25 baud=9600
#   led=blue on pin=1 podr=0040
#   ...
#   s1=1 s2=1
#   hb=0 podr=0040 s1=1 s2=1

# Determinism: two runs, identical UART + snapshot
cmp out/ek-ra6m5-run1/uart.log out/ek-ra6m5-run2/uart.log          # clean
cmp out/ek-ra6m5-run1/snapshot.json out/ek-ra6m5-run2/snapshot.json  # clean

cargo test -p labwired-core --test firmware_survival ek_ra6m5
#   1 passed (test_ek_ra6m5_smoke_survival)
```

## Notes

- **No new model.** The RA6M5 PORT, PFS/PWPR and SCI register layouts match
  the RA4M1 blocks already in the tree (`ra_port`, `ra_pfs`, the `sci` UART
  profile), so this slice is a descriptor + board + firmware package. The
  bases are from `R7FA6M5BH.h`.
- **Toolchain.** The RA6M5 is a Cortex-M33 (ARMv8-M Mainline), and the
  firmware is built for `thumbv8m.main-none-eabi`, so the compiler cannot
  emit DSP or FPU instructions. The unsupported-instruction audit runs on that
  ELF.
- **Clock.** The part resets onto the MOCO with every divider at /4 (FSP
  `bsp_feature.h`: `SCKSCR` 0x01, `SCKDIVCR` 0x22022222), which would leave
  PCLKA at 2 MHz. The firmware writes `SCKDIVCR = 0` under `PRCR.PRC0`, so
  PCLKA = 8 MHz. 9600 baud is the closest standard rate the plain BRR divider
  reaches from 8 MHz (+0.16 %). Zephyr's 115200 console needs the PLL.
- **Vector table size.** `cortex-m-rt` reserves 496 external vectors for
  ARMv8-M without a device crate, so `.text` starts at `0x800`. The RA6M5 has
  no option bytes in code flash (unlike RA4M1/RA2), so nothing sits in that
  range.
- `junit.xml` carries per-run wall-clock timings and is not part of the
  determinism claim; `uart.log` and `snapshot.json` are.
- The staged `configs/chips/onboarding/renesas-r7fa6m5b.yaml` and
  `renesas-ck_ra6m5.yaml` put code flash at `0x08000000`, which is the RA6M5's
  8 KiB data flash. They are kept, marked superseded, because
  `crates/core/src/bus/known_stubs.rs` still cites them.
