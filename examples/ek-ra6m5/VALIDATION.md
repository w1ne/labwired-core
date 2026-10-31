# Validation — EK-RA6M5 (R7FA6M5BH)

Claims, evidence, and what is explicitly **not** claimed yet. **SIM-DERIVED**:
there is no RA6M5 bench part, and no Renode parity claim is made.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF via the code-flash vector table (PC/SP init) | smoke-tested | run below: `Initial PC: 0x800, SP: 0x20080000` |
| Cortex-M33 core executing the committed firmware | smoke-tested | `labwired test` 3/3 checks, 4,000,000 steps |
| `PRCR`-guarded `SCKDIVCR` write (/1 on the reset MOCO) | stored only | `ra6_sysc` stores the write; no rate is derived from it, so the baud math in the firmware assumes the silicon result (`clk src=moco pclka=8000000 brr=25 baud=9600`) |
| FSP `bsp_clock_init` (MOSC, PLL, `SCKSCR` switch, FCACHE invalidate) | sim-tested | stock Zephyr runs it unpatched: `OSCSF` MOSCSF/PLLSF derived from the stop bits, FCACHEIV reads 0 (stub) |
| Clock generation (rates, stabilisation time) | **not modelled** | oscillators settle at once; `cpu_hz` stays 8 MHz after the switch to the PLL; the PRCR key is not checked |
| Stock Zephyr v4.0.0 `samples/hello_world` | sim-tested | `zephyr-hello.yaml` 1/1 (`*** Booting Zephyr OS build v4.0.0 ***`, `Hello World! ek_ra6m5/r7fa6m5bh3cfc`), `test_ek_ra6m5_zephyr_survival` |
| Stock Zephyr v4.0.0 `samples/synchronization` (SysTick timeout wake, semaphore, PendSV switch) | sim-tested | `zephyr-sync.yaml` 1/1, thread_a then thread_b in 21,110,044 steps |
| SCI0 FIFO mode (`FCR` FM + RFRST/TFRST handshake, `FTDRL` TX, `FDR`, `FRDRL` RX) | sim-tested | FSP `r_sci_uart_fifo_cfg` completes; Zephyr's console writes `FTDRL`; `test_sci_classic_fifo_mode`. No TX occupancy, trigger levels or FIFO interrupts |
| `MSTPCRB.MSTPB31` release of SCI0 | declared only | `MSTP` is a stub; SCI0 runs with the bit set too |
| SCI0 async TX at 9600 8N1 (`BRR 25` from 8 MHz) | smoke-tested | every console line, polled on `SSR.TDRE` |
| `PWPR` unlock + `PmnPFS` write (S1/S2 `PCR` pull-up) | smoke-tested | `s1=1 s2=1`: the pads read high through the PFS pull-up link |
| `PmnPFS` `PSEL`/`PMR` (P410/P411 → SCI0) | declared only | stored, no routing effect |
| PORT0 `PCNTR1` PDR/PODR, `PCNTR3` POSR/PORR, `PCNTR2` PIDR | smoke-tested | `led=blue on pin=1 podr=0040` … (one POSR/PORR store per LED; the whole-PODR read proves no other pin moved) |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| ICU / IELSR event linking, SCI interrupts | **not modelled** | the ICU is a stub; the smoke is polled |
| TrustZone (IDAU/SAU, CPSCU/PSCU) | **not modelled** | the image runs secure, as on silicon out of reset; `R_PSCU`/`R_CPSCU` are declared stubs so Zephyr's `soc_early_init_hook` can clear PSARB-E/ICUSARG-I; nothing checks attribution |
| DMAC/DTC, ELC, RTC, IWDT/WDT, USBFS, IIC0-2, CANFD, AGT0-5, USBHS, ETHERC/EDMAC, SPI0/1, GPT, ADC12, FACI, FCACHE, C-cache | declared stubs | `configs/chips/r7fa6m5bh.yaml` (`type: "stub"`) |
| SCI1-9, QSPI/OSPI, SDHI, SSIE, CTSU, CEC, DAC12, CRC, DOC, data flash, option-setting memory | **not mapped** | no window in `configs/chips/r7fa6m5bh.yaml`; an access faults with a memory violation. The loader warns and drops Zephyr's OFS segments at 0x0100A100+ |

## Commands and evidence (smoke verified 2026-10-30, Zephyr 2026-10-31)

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
#   2 passed (test_ek_ra6m5_smoke_survival, test_ek_ra6m5_zephyr_survival)

# Stock Zephyr v4.0.0 (ci/test.sh runs all three)
./target/release/labwired test --script examples/ek-ra6m5/zephyr-hello.yaml --no-uart-stdout
#   PASS  1/1 checks · zephyr-hello · 116387 steps
#   Audit (150,000 instructions): unsupported_total: 0
./target/release/labwired test --script examples/ek-ra6m5/zephyr-sync.yaml --no-uart-stdout
#   PASS  1/1 checks · zephyr-sync · 21110044 steps

# Fixture rebuild: two runs of build.sh, byte-identical
crates/firmware-ek-ra6m5-zephyr/build.sh
#   cb8edcdbca022b61c35846d85699e56db400f52fb82c71a14ca2ab6b6ef9cd89  r7fa6m5bh-zephyr-hello.elf
#   72d9e0071821238f251f1baa53e72bc1f58a940a086025cfa604014cd9557a15  r7fa6m5bh-zephyr-sync.elf

# Determinism: two zephyr-hello runs, identical uart.log + snapshot.json

# Negative controls (each makes test_ek_ra6m5_zephyr_survival fail):
#   - FCR keeps RFRST/TFRST set on read-back -> r_sci_uart_fifo_cfg spins
#   - SYSTEM back on ra_sysc (no MOSCSF/PLLSF) -> bsp_clock_init spins
```

## Notes

- **Reused models.** The RA6M5 PORT, PFS/PWPR and SCI register layouts match
  the RA4M1 blocks already in the tree (`ra_port`, `ra_pfs`, the `sci` UART
  profile). The bases are from `R7FA6M5BH.h`. For Zephyr, the SYSTEM window
  moved to `ra6_sysc` (the RA8M1's full-register-file model with an RA6
  layout) and the `sci` profile gained the FIFO registers.
- **Zephyr version.** Zephyr v3.7.0 has no `ek_ra6m5` board, so the fixtures
  are Zephyr v4.0.0, the first release with it. The other Zephyr fixtures in
  the tree stay on v3.7.0.
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
