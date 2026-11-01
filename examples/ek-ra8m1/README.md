# EK-RA8M1 — LabWired example

The [EK-RA8M1](https://www.renesas.com/en/design-resources/boards-kits/ek-ra8m1)
evaluation kit: an **R7FA8M1AHECBD** (RA8M1, Arm Cortex-M85 with Helium and
TrustZone, 480 MHz max, 2 MiB code flash, 896 KiB user SRAM). This is the
**UART+LED L1 slice**; the rest of the SoC is declared stubs or not mapped.

## What runs here

A bare-register smoke firmware (`crates/firmware-ek-ra8m1-smoke`) that:

- stays on the reset **MOCO (8 MHz)** with every `SCKDIVCR` divider at /1 (the
  RA8M1 reset value); it never starts HOCO or the PLLs,
- switches the SCI kernel clock **SCICLK** to MOCO /1 under `PRCR.PRC0`, with
  the FSP `bsp_peripheral_clock_set` handshake (`SCICKCR.CKSREQ` = 1, wait for
  `CKSRDY` = 1, write source and divider, clear `CKSREQ`, wait for
  `CKSRDY` = 0),
- releases SCI9 from module stop (`MSTPCRB.MSTPB22 = 0`),
- unlocks `PWPRS` (the secure write-protect register; this image runs secure)
  and routes **PA14** (TXD9) / **PA15** (RXD9) to SCI9 (`PmnPFS.PMR = 1`,
  `PSEL = 0b00101`), and turns on the **S1/S2** pull-ups (`PmnPFS.PCR`), then
  locks `PWPRS` again,
- brings **SCI9** (SCI_B) up async 8N1 at **9600** baud in the FSP
  `r_sci_b_uart.c` order: `CCR0` off, `CFCLR` clear-all, `CCR1.SPB2DT`,
  `CCR2.BRR = 8 MHz / (32 × 9600) − 1 = 25` (9615 baud, +0.16 %), `CCR3` 8-bit
  LSB-first, then `CCR0.TE|RE` and a wait for `CESR.TIST|RIST`,
- prints the `SCICKCR`, `CCR2.BRR`, `CCR3` and `CESR` it read back,
- walks **LED1 blue P600, LED2 green P414, LED3 red P107** (active high)
  through `PCNTR3` POSR/PORR and prints the pin and the whole `PODR` of its
  port read back from `PCNTR1`,
- prints the S1/S2 levels (released = 1 on the pull-up), then heartbeats.

**Console baud.** Zephyr's `ek_ra8m1` console runs at 115200 on a PLL-derived
SCICLK. 115200 cannot be hit accurately from the 8-MHz MOCO without the `MDDR`
modulator, so this smoke uses **9600 8N1**. Set the J-Link OB virtual COM
port to 9600 on silicon.

**Toolchain.** The firmware is built for `thumbv8m.main-none-eabi`: Armv8-M
Mainline code with no FPU, Helium/MVE or low-overhead-branch instructions,
which is the subset the engine decodes. It is not built with
`-mcpu=cortex-m85`.

**Stock Zephyr.** Unmodified Zephyr v3.7.0 `samples/hello_world` and
`samples/synchronization` for board `ek_ra8m1` also run
(`zephyr-hello.yaml`, `zephyr-sync.yaml`; fixtures
`tests/fixtures/r7fa8m1ah-zephyr-{hello,sync}.elf`, rebuilt byte-identical by
`crates/firmware-ek-ra8m1-zephyr/build.sh`). Zephyr builds for
`-mcpu=cortex-m85+nomve`, so these images do use the Armv8.1-M `DLS`/`LE` and
`CSINC` instructions, and they run FSP's full `bsp_clock_init` (main
oscillator, PLL1/PLL2, ICLK on PLL1P) before the SCI9 console comes up at
115200.

Stock Zephyr v4.0.0 `samples/drivers/counter/alarm`
(`zephyr-counter-alarm.yaml`, fixture
`tests/fixtures/r7fa8m1ah-zephyr-counter-alarm.elf`) runs four AGT0
compare-match alarms; each one reaches the core through the ICU `IELSRn`
event link (`ra_icu` + `ra_agt`).

Stock Zephyr v3.7.0 `tests/drivers/uart/uart_basic_api`
(`zephyr-uart-basic-api.yaml`, fixture
`tests/fixtures/r7fa8m1ah-zephyr-uart-basic-api.elf`) passes 7/7 on the
interrupt-driven SCI_B driver: SCI9's RXI/TXI/TEI reach the core through
`IELSR24-26`.

## Build and run

```bash
# from the repository root
cargo build -p firmware-ek-ra8m1-smoke --release --target thumbv8m.main-none-eabi

./target/release/labwired test \
  --script examples/ek-ra8m1/io-smoke.yaml \
  --output-dir out/ek-ra8m1-smoke --no-uart-stdout
```

Interactive run (UART to stdout):

```bash
./target/release/labwired \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-ek-ra8m1-smoke \
  --system configs/systems/ek-ra8m1.yaml \
  --max-steps 400000
```

Expected output (first beats):

```text
EK-RA8M1 SMOKE
cpu=cortex-m85 uart=sci9 (sci_b)
clk src=moco sciclk=8000000 scickcr=01 brr=25 baud=9600
ccr3=00001200 cesr=11
led=blue on pin=1 port=6 podr=0001
led=blue off pin=0 port=6 podr=0000
led=green on pin=1 port=4 podr=4000
led=green off pin=0 port=4 podr=0000
led=red on pin=1 port=1 podr=0080
led=red off pin=0 port=1 podr=0000
s1=1 s2=1
hb=0 podr6=0001 s1=1 s2=1
...
```

## Board facts used

| Item | Value | Source |
| :--- | :--- | :--- |
| MCU | R7FA8M1AHECBD (2 MiB code flash, 896 KiB SRAM, 12 KiB data flash, 64 KiB ITCM + 64 KiB DTCM) | Zephyr `ra8x1.dtsi`, `r7fa8m1xh.dtsi`; Renode `renesas-r7fa8m1a.repl` |
| Core / reset clock | Cortex-M85; MOCO 8 MHz at reset, every divider /1 (`SCKDIVCR` 0) | FSP `ra8m1/bsp_feature.h`; Zephyr `r7fa8m1xh.dtsi` (`clock-moco` 8 MHz) |
| Flash / RAM | 2 MiB @ 0x02000000 / 896 KiB @ 0x22000000 | FSP `BSP_FEATURE_FLASH_CODE_FLASH_START`; Zephyr `ra8x1.dtsi` |
| LEDs | LED1 blue P600, LED2 green P414, LED3 red P107, active high | Zephyr `ek_ra8m1.dts` |
| Switches | S1 P009, S2 P008, active low, pull-up | Zephyr `ek_ra8m1.dts` |
| Console UART | SCI9 PA14 (TX) / PA15 (RX), SCI_B | Zephyr `ek_ra8m1-pinctrl.dtsi`, `zephyr,console = &uart9`, `renesas,ra8-uart-sci-b` |
| SCI9 clock / gate | SCICLK (`SCICKCR`/`SCICKDIVCR`), `MSTPCRB` bit 22 | FSP `bsp_clocks.c`, `bsp_module_stop.h` |

## Explicitly not claimed

- **Cortex-M85 extensions.** Helium/MVE, the tail-predicated loops
  (`DLSTP`/`WLSTP`/`LETP`/`LCTP`), PACBTI, `BF*` and the M85 caches are not
  modelled. The scalar `DLS`/`WLS`/`LE` and `CSEL` family are.
- **TrustZone.** No security state; the non-secure peripheral aliases
  (+0x10000000) are not mapped.
- **Clock rates.** The SYSTEM window stores every register, derives `OSCSF`
  from the oscillator stop bits and answers the xxxCKCR handshakes, but the
  oscillators settle at once and no rate is derived: `cpu_hz` stays 8 MHz
  after Zephyr switches ICLK to the 480 MHz PLL1P, and `PRCR` gates nothing.
- **Module stop.** `MSTP` is a stub: SCI9 runs whether or not `MSTPB22` is
  cleared.
- **ICU / IELSR.** Both ICU windows are stubs, so no peripheral event reaches
  the NVIC; the smoke polls `CSR.TDRE` and the Zephyr console is polled too.
- **SCI_B beyond polled async.** FIFO mode, `CCR0` interrupt enables, error
  flags, the simple SPI/I2C/LIN/Manchester modes and wire timing.
- **PFS peripheral mux.** `PSEL`/`PMR` are stored only; SCI9 TX does not depend
  on them.
- **DMAC/DTC, FCACHE, FACI, ELC, RTC, IWDT/WDT, AGT, USBFS/USBHS, GPT,
  ETHERC/EDMAC, SPI0/1, CANFD** are declared stubs (ADC0/ADC1 are modelled
  since the stock Zephyr `adc_api` slice and IIC0/IIC1 since the stock
  Zephyr `bme280` slice; see VALIDATION.md) (reads return 0, writes are
  dropped).
- **SCI0-8, OSPI/xSPI, SDRAM, SDHI, SSIE, CEU, DAC, CRC, DOC, RSIP-E51A,
  option-setting memory, standby SRAM** and every other block are **not
  mapped**. Firmware that touches them faults with a memory violation.

See `VALIDATION.md` for the tier table.
