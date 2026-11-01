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
| Stock Zephyr v4.0.0 `samples/subsys/shell/shell_module` (interrupt-driven shell serial backend on SCI0; the sample of Renode's `shell_module.robot`) | sim-tested | `zephyr-shell.yaml` 2/2 in 443,990 steps: typed `demo ping` → `pong`, `kernel version`, `device list`, `demo params a b` (argc/argv), TAB completion (`kern<TAB>up<TAB>` → `kernel uptime`, `Uptime: 1249 ms`) and history recall (4× ESC [ A → `kernel version`). Negative control: without sci0 `elc_event` it prints nothing |
| SCI0 interrupt events (`SCR` TIE/RIE/TE/RE/TEIE; TXI on the TDRE edge of a timed shift register, TEI one frame after the last byte, RXI per character a frame apart) | sim-tested | `ra6m5_sci_icu` 6/6 in both scheduler modes; 5 mutations caught (SCR not stored, RIE or TIE/TEIE mis-mapped, SSR_FIFO off, RTRG ignored). No ERI or error flags; the frame time is the chip YAML `baud`, not `SMR`/`BRR`/`SEMR` |
| `MSTPCRB.MSTPB31` release of SCI0 | declared only | `MSTP` is a stub; SCI0 runs with the bit set too |
| SCI0 async TX at 9600 8N1 (`BRR 25` from 8 MHz) | smoke-tested | every console line, polled on `SSR.TDRE` |
| `PWPR` unlock + `PmnPFS` write (S1/S2 `PCR` pull-up) | smoke-tested | `s1=1 s2=1`: the pads read high through the PFS pull-up link |
| `PmnPFS` `PSEL`/`PMR` (P410/P411 → SCI0) | declared only | stored, no routing effect for SCI0 (they route P405/P406 to the GPT, below) |
| PORT0 `PCNTR1` PDR/PODR, `PCNTR3` POSR/PORR, `PCNTR2` PIDR | smoke-tested | `led=blue on pin=1 podr=0040` … (one POSR/PORR store per LED; the whole-PODR read proves no other pin moved) |
| Determinism (same inputs → same output) | smoke-tested | two runs, identical `uart.log` and `snapshot.json` |
| Instruction support for this firmware | audited | `unsupported_instruction_audit.sh`: 200,000 instructions, 0 unsupported |
| SCI interrupts (RXI/TXI/TEI/ERI as ELC events) | **not modelled** | SCI0 raises no events, so the smoke and the Zephyr console are polled |
| Stock Zephyr v4.1.0 `samples/basic/button` (S1 on P005 → IRQ10 → ELC event 0x00B → ICU IELSR42 → NVIC line 42) | sim-tested | `zephyr-button.yaml` 5/5: two presses give exactly two `Button pressed at` lines (cycles 500,060,451 and 1,499,740,451), the releases none, and LED1 (P006) mirrors S1 (4 edges, ends low); 9,874,303 steps; audit 2,000,000 steps, 0 unsupported. Without `irq_pins` on `icu` it prints nothing after "Press the button" |
| ICU IRQ-pin half (`ra_icu`: `IRQCRn.IRQMD` edge, `PmnPFS.ISEL` gate, `ICU_IRQn` = event n + 1; pad → IRQn table from Zephyr `r7fa6m5xh.dtsi`) | sim-tested | the button above; unit tests `peripherals::ra_icu` |
| ICU NMI, `IRQCRn` digital filter, low-level re-raise, `DTCE`; AGT pulse output, event counter, measurement modes | **not modelled** | stored only; an IRQ edge is taken at once, low-level mode raises on the falling edge only |
| `PmnPFS` `PDR`/`PODR` → `PCNTR1` (the same flip-flops on silicon) | sim-tested | Stock Zephyr v4.1.0 `samples/basic/blinky` (`zephyr-blinky.yaml` PASS 4/4): P006 made an output by its PmnPFS store and toggled five times through the 16-bit `PCNTR1.PODR` alias, exactly five `LED state` lines ending OFF, 6 edges on `port0:6`, `PCNTR1` ends PDR set / PODR clear; 39,715 steps; audit 1,000,000 steps, 0 unsupported; fixture sha256 `f6826f74e095d97e…`. The button sample's LED1 mirror of S1 now shows 4 edges and ends low (9,874,303 steps). Without the PFS → port hand-over both scripts fail. On RA6M5 the 16-bit `PCNTR1` aliases are PODR at +0x0 and PDR at +0x2 (FSP `R7FA6M5BH.h`, `swapped_half_aliases`); with natural half order the blinky toggles PDR and fails, and the button LED never lights. Unit test `pins::pad_control::tests::ra_pfs_low_byte_stores_write_the_port_output`, `peripherals::gpio::tests::ra_port_16_bit_aliases_follow_the_part`; integration `pad_control_pulls::ra_pfs_pdr_podr_are_the_port_output_flip_flops` |
| `PmnPFS` read-back of PDR/PODR after `PCNTR1`/`PCNTR3` writes; `PmnPFS.PIDR` | **not modelled** | PmnPFS returns the last store; PIDR there reads 0 |
| SCI ERI and the receive error flags (`ORER`/`FER`/`PER`) | **not modelled** | ERI (event 0x183) is never raised; RXI/TXI/TEI are covered above |
| TrustZone (IDAU/SAU, CPSCU/PSCU) | **not modelled** | the image runs secure, as on silicon out of reset; `R_PSCU`/`R_CPSCU` are declared stubs so Zephyr's `soc_early_init_hook` can clear PSARB-E/ICUSARG-I; nothing checks attribution |
| Stock Zephyr v4.1.0 `tests/drivers/adc/adc_api` (ADC1 channels 0 and 2, scan end → ELC event 0x166 → ICU IELSR39 → NVIC line 39) | smoke-tested | `zephyr-adc-api.yaml` PASS 2/2: `adc_basic` 6/6 (`asynchronous_call`, `invalid_request`, `repeated_samplings`, `sample_one_channel`, `sample_two_channels`, `sample_with_interval`), the buffers holding `0x07ff` (potentiometer, 1650 mV) and `0x03a2` (TMP36 at 25 °C, 750 mV) from `adc-system.yaml`; 316,024 steps; audit 2,000,000 steps, 0 unsupported; fixture sha256 `9a1ad5b6c1622…` |
| ADC0/ADC1 (`ra_adc12`, the RA8M1 model at the RA6M5 bases 0x4017_0000 / 0x4017_0200): software-started single and continuous scans, `ADDRn`, `ADCER.ADPRC`/`ADRFMT`, addition/averaging, scan-end ELC events 0x160/0x166 (FSP ra6m5 `bsp_elc.h`) | sim-tested | the test above; `crates/core/tests/ra6m5_adc12_icu.rs` (vectors through IELSR39 with the converted codes, an unlinked scan end pends nothing) in both scheduler modes; unit tests `peripherals::ra_adc12` |
| ADC12 conversion time, group scan, double trigger, hardware/ELC start triggers, temperature sensor and internal reference, window comparator, `ADBUF`, sample-and-hold, `ADCER.ACE` clear-after-read | **not modelled** | each channel converts in a nominal 1 µs at the 200 MHz guest clock; a group-scan start converts nothing; `ADDRn` keep their value after a read |
| Stock Zephyr v4.1.0 `samples/sensor/bme280` (BME280 at 0x76 on IIC1; RXI/TXI/TEI/ERI → ELC events 0x078-0x07B → ICU IELSR91-94) | smoke-tested | `zephyr-bme280.yaml` PASS 2/2: the driver's chip-ID check, calibration load and once-a-second RTIO fetches (register burst reads with a restart, register writes) all complete; `temp: 25.0; press: 101.324218; humidity: 50.3906` twice from `bme280-system.yaml`'s defaults, then `temp: 21.500000; press: 98.996093; humidity: 39.995117` twice after the 1.5 s stimulus to 21.5 °C / 990 hPa / 40 %RH; 296,579 steps; audit 2,000,000 steps, 0 unsupported; fixture sha256 `4a406198ff35036e…` |
| IIC0-2 (`ra_iic`): RIIC master transmit/receive, 7-bit addresses, `ST`/`RS`/`SP` conditions with `BBSY`/`MST`/`TRS`, `ICDRT` holding register behind the shifter, read turn-around with the dummy `ICDRR` read, stop/restart deferred until the last byte is read, address NACK (`NACKF`/`ACKBR`), `ACKBT` behind `ACKWP`, `IICRST` resets; events on the rising edge of flag & enable; wire time from `ICBRL`/`ICBRH`/`CKS` | sim-tested | the sample above; `crates/core/tests/ra6m5_iic_icu.rs` (the start's TXI vectors through IELSR92; the BME280 chip-ID read 0xD0 → 0x60 step by step on the ICU events; an absent device NACKed through ERI) in both scheduler modes; unit tests `peripherals::ra_iic` |
| RIIC slave mode (`SARLn`/`SARUn`/`ICSER`), 10-bit addresses, a NACKed data byte, arbitration loss, SCL-low timeout (`TMOF`), SMBus, wake-up (`ICWUR`), `RDRFS`, `WAIT` hold timing, `ICCR1` SCL/SDA overrides and `CLO`, `ICMR1.BC`, DTC transfers | **not modelled** | slave registers stored only; devices ACK every data byte; rise/fall times are not added to the SCL period |
| Stock Zephyr v4.1.0 `tests/drivers/pwm/pwm_api` (pwm1 = GPT1, GTIOC1A on P405 through PmnPFS PMR/PSEL 00011) | smoke-tested | `zephyr-pwm-api.yaml` PASS 5/5: `pwm_basic` 2/2 (`test_pwm_cycle`, `test_pwm_nsec`: 50%, 100%, 0% for a second each), 4,128 edges on P405 (50% of 64,000 PCLKD counts = 64,000/64,000 bus cycles, 3,125 edges; 50% of 2 ms = 200,000/200,000, 1,001 edges; 100% high, 0% low), final `GTPR` 199,999 and `GTUDDTYC.OADTY` 0b10; 131,375 steps, 0.01 s; audit 2,000,000 steps, 0 unsupported |
| GPT0-9 (`ra_gpt`): saw-wave up-count at PCLKD / 2^`GTCR.TPCS`, `GTSTR`/`GTSTP`/`GTCLR` channel masks gated by `GTSSR`/`GTPSR`/`GTCSR` bit 31, cycle end (`TCFPO`, `GTPBR` → `GTPR`, `GTCCRC` → `GTCCRA`, `GTCCRE` → `GTCCRB` per `GTBER`, `CCRSWT` force), compare A/B (`TCFA`/`TCFB`), events 0x0C0 + 9n (+0 CCMPA, +1 CCMPB, +6 OVF), GTIOCnA/B from `GTIOR` (cycle-end / compare-match levels, initial level for the first period, `OADFLT`/`OAHLD` when stopped, `OAE`) and `GTUDDTYC` 0%/100%, edges at their exact cycle | smoke-tested | the test above; `crates/core/tests/ra6m5_gpt_pwm.rs` (P405 carries GTIOC1A only while P405PFS has PMR = 1 and PSEL = 00011; 30 of 100 counts high per period after an initial-level first period; GPT1 overflow latches IELSR64 once per period) in both scheduler modes; unit tests `peripherals::ra_gpt` |
| GPT triangle-wave and one-shot modes, down-counting, input capture, hardware start/stop/clear/count sources, compare C-F events and double buffers, dead time, `GTINTAD`/POEG output disable, A/D triggers, `GTWP` write protection, `OADTY` cycle-end timing | **not modelled** | `GTCR.MD` != 0 counts like saw-wave with the pins held; `GTWP` stored only; `OADTY` applies at once |
| DMAC/DTC, ELC, RTC, IWDT/WDT, USBFS, CANFD, USBHS, ETHERC/EDMAC, SPI0/1, FACI, FCACHE, C-cache | declared stubs | `configs/chips/r7fa6m5bh.yaml` (`type: "stub"`) |
| SCI1-9, QSPI/OSPI, SDHI, SSIE, CTSU, CEC, DAC12, CRC, DOC, data flash, option-setting memory | **not mapped** | no window in `configs/chips/r7fa6m5bh.yaml`; an access faults with a memory violation. The loader warns and drops Zephyr's OFS segments at 0x0100A100+ |

## Commands and evidence (smoke verified 2026-10-30, Zephyr 2026-10-31, counter alarm, SCI interrupts, button, blinky, adc_api, bme280 and pwm_api 2026-11-01)

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

# Stock Zephyr v4.1.0 button (P005 -> IRQ10 -> ICU), two S1 presses
./target/release/labwired test --script examples/ek-ra6m5/zephyr-button.yaml --no-uart-stdout
#   PASS  5/5 checks · zephyr-button · 9874303 steps
#   Set up button at gpio@40080000 pin 5 / Set up LED at gpio@40080000 pin 6
#   Press the button
#   Button pressed at 500060451
#   Button pressed at 1499740451
# Stock Zephyr v4.0.0 tests/drivers/uart/uart_basic_api (SCI0 RXI/TXI/TEI + ICU)
./target/release/labwired test --script examples/ek-ra6m5/zephyr-uart-basic-api.yaml --no-uart-stdout
#   PASS  2/2 checks · zephyr-uart-basic-api · 45082111 steps
#   Audit (150,000 instructions, boot only): unsupported_total: 0
cargo test -p labwired-core --test ra6m5_sci_icu
#   6 passed

#   LED1 (port0:6): 4 edges; PCNTR1 ends PDR6 set, PODR6 clear

# Stock Zephyr v4.1.0 blinky (P006 via PmnPFS.PDR, 16-bit PCNTR1.PODR at +0x0)
./target/release/labwired test --script examples/ek-ra6m5/zephyr-blinky.yaml --no-uart-stdout
#   PASS  4/4 checks · zephyr-blinky · 39715 steps
#   LED state: OFF / ON / OFF / ON / OFF; 6 edges on port0:6
#   Audit (1,000,000 instructions): unsupported_total: 0
cargo test -p labwired-core --test pad_control_pulls
#   5 passed

# Stock Zephyr v4.1.0 adc_api (ADC1 ch0 pot + ch2 TMP36, scan end via IELSR39)
./target/release/labwired test --script examples/ek-ra6m5/zephyr-adc-api.yaml --no-uart-stdout
#   PASS  2/2 checks · zephyr-adc-api · 316024 steps
#   Samples read: 0x07ff 0x03a2 0x8000 0x8000 0x8000 0x8000
#   SUITE PASS - 100.00% [adc_basic]: pass = 6, fail = 0, skip = 0, total = 6
#   Audit (2,000,000 instructions): unsupported_total: 0
cargo test -p labwired-core --test ra6m5_adc12_icu   # also --features event-scheduler
#   2 passed

# Stock Zephyr v4.1.0 samples/sensor/bme280 (BME280 at 0x76 on IIC1, IELSR91-94)
./target/release/labwired test --script examples/ek-ra6m5/zephyr-bme280.yaml --no-uart-stdout
#   PASS  2/2 checks · zephyr-bme280 · 296579 steps
#   temp: 25.0; press: 101.324218; humidity: 50.3906                 (x2)
#   temp: 21.500000; press: 98.996093; humidity: 39.995117           (after the stimulus)
#   Audit (2,000,000 instructions): unsupported_total: 0
cargo test -p labwired-core --test ra6m5_iic_icu   # also --features event-scheduler
#   3 passed

# Stock Zephyr v4.1.0 tests/drivers/pwm/pwm_api (pwm1 = GPT1, GTIOC1A on P405)
./target/release/labwired test --script examples/ek-ra6m5/zephyr-pwm-api.yaml --no-uart-stdout
#   PASS  5/5 checks · zephyr-pwm-api · 131375 steps
#   SUITE PASS - 100.00% [pwm_basic]: pass = 2, fail = 0, skip = 0, total = 2
#   P405: 4128 edges (64000 / 64000 cycles, then 200000 / 200000)
#   Audit (2,000,000 instructions): unsupported_total: 0
cargo test -p labwired-core --test ra6m5_gpt_pwm   # also --features event-scheduler
#   3 passed

# Fixture rebuild: two runs of build.sh, byte-identical
crates/firmware-ek-ra6m5-zephyr/build.sh
#   cb8edcdbca022b61c35846d85699e56db400f52fb82c71a14ca2ab6b6ef9cd89  r7fa6m5bh-zephyr-hello.elf
#   72d9e0071821238f251f1baa53e72bc1f58a940a086025cfa604014cd9557a15  r7fa6m5bh-zephyr-sync.elf
#   5a2167b96f644d6f5c4706e940a00a3e8483eef7e5759525a4ac269fbe1b569e  r7fa6m5bh-zephyr-counter-alarm.elf
#   f2041b1a3a59371236ac58f3ab65d14cd34b922f53683e1ab528667419bc6ce2  r7fa6m5bh-zephyr-button.elf
#   4060e1969d11755f583079c69c9c7b2c85e36d4c8ade5168b256f0847a45def9  r7fa6m5bh-zephyr-uart-basic-api.elf
#   f6826f74e095d97eacfc160c57b6574e3e4f4143572e1d2743079a5432ce2627  r7fa6m5bh-zephyr-blinky.elf
#   9a1ad5b6c16223c3a8f94008721265398046b66143d9b0ea2ba6e785781d5c82  r7fa6m5bh-zephyr-adc-api.elf
#   4a406198ff35036e14562b8a330703a5eae91d22a3b5eca05e4a3166b0eb8698  r7fa6m5bh-zephyr-bme280.elf
#   b5eb516d740f66287f67a27de4a31f026c97df4c197acf6bf7bbc9468dcd75ba  r7fa6m5bh-zephyr-pwm-api.elf

# Determinism: two zephyr-hello runs, two zephyr-counter-alarm runs and two
# zephyr-button runs, two zephyr-blinky runs, two zephyr-adc-api runs,
# two zephyr-bme280 runs, two zephyr-pwm-api runs (and logic edges),
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
# Negative controls for zephyr-blinky.yaml and the LED half of
# zephyr-button.yaml:
#   - port ignores PmnPFS PDR/PODR stores -> P006 stays an input: blinky
#     has no edges and PCNTR1 reads 0, the button LED none (both FAIL)
#   - natural 16-bit half order (PDR at +0x0) -> blinky toggles PDR and ends
#     with P006 an input (memory_value FAILs); the button LED never lights
# Negative control for zephyr-bme280.yaml: no BME280 on IIC1 -> address
# NACK, FSP abort, `ID read failed: -5`, `Device "bme280@76" is not ready`
# (FAIL)
# Negative control for zephyr-pwm-api.yaml: GPT back to the old `stub`
# window -> pwm_basic still passes 2/2 (the driver only writes registers),
# but P405 has 0 edges and GTPR/GTUDDTYC read 0 (3 of 5 checks FAIL)
# Negative control for zephyr-adc-api.yaml: ADC0/ADC1 back to the old
# `stub` window -> no scan end, test_adc_asynchronous_call fails
# (`k_poll failed with error -11`)
```

## Notes

- **Reused models.** The RA6M5 PORT, PFS/PWPR and SCI register layouts match
  the RA4M1 blocks already in the tree (including the upper-half-first 16-bit
  PCNTRn aliases) (`ra_port`, `ra_pfs`, the `sci` UART
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
