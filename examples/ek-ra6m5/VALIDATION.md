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
| Stock Zephyr v4.1.0 `samples/drivers/counter/alarm` (AGT0 compare match A → ELC event 0x041 → ICU IELSR84 → NVIC line 84) | sim-tested | `zephyr-counter-alarm.yaml` 1/1: four alarms (Now: 2/6/14/30), then the 65536-tick request refused, in 236,814 steps (6 G cycles, idle-skipped); `ra6m5_agt_icu` 3/3 |
| ICU `IELSRn` event link (`ra_icu`: event → `IR` → line n as a level, `IR` W0C) | sim-tested | `ra6m5_agt_icu`: an unlinked event reaches no line; clearing `IR` drops the pend |
| AGT0-5 16-bit timer mode (`ra_agt`: LOCO/PCLKB count source, underflow, compare A/B) | sim-tested | the counter alarm runs on AGT0 (LOCO / 16 = 2048 Hz); AGT1-5 share the model, unexercised by firmware |
| SCI0 FIFO mode (`FCR` FM + RFRST/TFRST handshake, `FTDRL` TX, `FDR`, `FRDRL` RX, `SSR_FIFO` TDFE/TEND/RDF/DR) | sim-tested | FSP `r_sci_uart_fifo_cfg` completes; Zephyr's console writes `FTDRL`; `test_sci_classic_fifo_mode`; `ra6m5_sci_icu` checks RDF at `RTRG` and DR below it. No TX occupancy (`FDR.T` reads 0); DR is set at once, not after 15 ETU |
| Stock Zephyr v4.0.0 `tests/drivers/uart/uart_basic_api` (SCI0 RXI/TXI/TEI → ELC events 0x180-0x182 → ICU IELSR0-2 → NVIC lines 0-2) | sim-tested | `zephyr-uart-basic-api.yaml` 2/2: uart_basic_api 6/6 + uart_basic_api_pending 1/1 in 45,082,111 steps (300 M cycles, idle-skipped). Negative control: the same ELF with sci0 lacking `elc_event` fails fifo_fill and hangs in fifo_read |
| SCI0 interrupt events (`SCR` TIE/RIE/TE/RE/TEIE; TXI on the TDRE edge of a timed shift register, TEI one frame after the last byte, RXI per character a frame apart) | sim-tested | `ra6m5_sci_icu` 6/6 in both scheduler modes; 5 mutations caught (SCR not stored, RIE or TIE/TEIE mis-mapped, SSR_FIFO off, RTRG ignored). No ERI or error flags; the frame time is the chip YAML `baud`, not `SMR`/`BRR`/`SEMR` |
| `MSTPCRB.MSTPB31` release of SCI0 | declared only | `MSTP` is a stub; SCI0 runs with the bit set too |
| SCI0 async TX at 9600 8N1 (`BRR 25` from 8 MHz) | smoke-tested | every console line, polled on `SSR.TDRE` |
| `PWPR` unlock + `PmnPFS` write (S1/S2 `PCR` pull-up) | smoke-tested | `s1=1 s2=1`: the pads read high through the PFS pull-up link |
| `PmnPFS` `PSEL`/`PMR` (P410/P411 → SCI0) | declared only | stored, no routing effect |
| PORT0 `PCNTR1` PDR/PODR, `PCNTR3` POSR/PORR, `PCNTR2` PIDR | smoke-tested | `led=blue on pin=1 podr=0040` … (one POSR/PORR store per LED; the whole-PODR read proves no other pin moved) |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| SCI interrupts (RXI/TXI/TEI/ERI as ELC events) | **not modelled** | SCI0 raises no events, so the smoke and the Zephyr console are polled |
| ICU IRQ-pin half (`IRQCRn`, NMI), `DTCE`; AGT pulse output, event counter, measurement modes | **not modelled** | stored only |
| TrustZone (IDAU/SAU, CPSCU/PSCU) | **not modelled** | the image runs secure, as on silicon out of reset; `R_PSCU`/`R_CPSCU` are declared stubs so Zephyr's `soc_early_init_hook` can clear PSARB-E/ICUSARG-I; nothing checks attribution |
| DMAC/DTC, ELC, RTC, IWDT/WDT, USBFS, IIC0-2, CANFD, USBHS, ETHERC/EDMAC, SPI0/1, GPT, ADC12, FACI, FCACHE, C-cache | declared stubs | `configs/chips/r7fa6m5bh.yaml` (`type: "stub"`) |
| SCI1-9, QSPI/OSPI, SDHI, SSIE, CTSU, CEC, DAC12, CRC, DOC, data flash, option-setting memory | **not mapped** | no window in `configs/chips/r7fa6m5bh.yaml`; an access faults with a memory violation. The loader warns and drops Zephyr's OFS segments at 0x0100A100+ |

## Commands and evidence (smoke verified 2026-10-30, Zephyr 2026-10-31, counter alarm 2026-11-01)

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

# Stock Zephyr v4.1.0 counter alarm (AGT0 + ICU)
./target/release/labwired test --script examples/ek-ra6m5/zephyr-counter-alarm.yaml --no-uart-stdout
#   PASS  1/1 checks · zephyr-counter-alarm · 236814 steps
#   Audit (150,000 instructions): unsupported_total: 0
cargo test -p labwired-core --test ra6m5_agt_icu
#   3 passed

# Fixture rebuild: two runs of build.sh, byte-identical
crates/firmware-ek-ra6m5-zephyr/build.sh
#   cb8edcdbca022b61c35846d85699e56db400f52fb82c71a14ca2ab6b6ef9cd89  r7fa6m5bh-zephyr-hello.elf
#   72d9e0071821238f251f1baa53e72bc1f58a940a086025cfa604014cd9557a15  r7fa6m5bh-zephyr-sync.elf
#   5a2167b96f644d6f5c4706e940a00a3e8483eef7e5759525a4ac269fbe1b569e  r7fa6m5bh-zephyr-counter-alarm.elf

# Determinism: two zephyr-hello runs and two zephyr-counter-alarm runs,
# identical uart.log + snapshot.json

# Negative controls (each makes test_ek_ra6m5_zephyr_survival fail):
#   - FCR keeps RFRST/TFRST set on read-back -> r_sci_uart_fifo_cfg spins
#   - SYSTEM back on ra_sysc (no MOSCSF/PLLSF) -> bsp_clock_init spins
# Negative controls for zephyr-counter-alarm.yaml (each FAILs it, the run
# stops after "Set alarm in 2 sec"):
#   - the chip descriptor from main (ICU and AGT stubs)
#   - ICU back to `type: stub`, AGT0-5 still ra_agt
# Mutations caught by ra6m5_agt_icu (2 of 3 tests fail each):
#   - ICU back to `type: stub`
#   - AGT0 elc_event 64 -> 67 (AGT1's INT)
```

## Notes

- **Reused models.** The RA6M5 PORT, PFS/PWPR and SCI register layouts match
  the RA4M1 blocks already in the tree (`ra_port`, `ra_pfs`, the `sci` UART
  profile). The bases are from `R7FA6M5BH.h`. For Zephyr, the SYSTEM window
  moved to `ra6_sysc` (the RA8M1's full-register-file model with an RA6
  layout) and the `sci` profile gained the FIFO registers.
- **Zephyr version.** Zephyr v3.7.0 has no `ek_ra6m5` board, so the fixtures
  are Zephyr v4.0.0, the first release with it. The other Zephyr fixtures in
  the tree stay on v3.7.0. The counter alarm is v4.1.0: v4.0.0 has the
  Renesas AGT counter driver but no `agt` nodes in the RA6 devicetree.
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
