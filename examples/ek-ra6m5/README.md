# EK-RA6M5 — LabWired example

The [EK-RA6M5](https://www.renesas.com/en/design-resources/boards-kits/ek-ra6m5)
evaluation kit: an **R7FA6M5BH3CFC** (RA6M5, Cortex-M33 with TrustZone,
200 MHz max, 2 MiB code flash, 512 KiB SRAM). This is the **UART+LED L1
slice**; the rest of the SoC is declared stubs or not mapped.

## What runs here

A bare-register smoke firmware (`crates/firmware-ek-ra6m5-smoke`) that:

- unlocks `PRCR.PRC0` and sets `SCKDIVCR` to /1 on the reset **MOCO (8 MHz)**;
  it never starts HOCO or the PLL,
- releases SCI0 from module stop (`MSTPCRB.MSTPB31 = 0`),
- unlocks `PWPR` and routes **P411** (TXD0) / **P410** (RXD0) to SCI0
  (`PmnPFS.PMR = 1`, `PSEL = 0b00100`), and turns on the **S1/S2** pull-ups
  (`PmnPFS.PCR`), then locks `PWPR` again,
- brings **SCI0** up async 8N1 at **9600** baud: `BRR = 8 MHz / (32 × 9600) − 1
  = 25` (9615 baud, +0.16 %),
- walks **LED1 blue P006, LED2 green P007, LED3 red P008** (active high)
  through `PCNTR3` POSR/PORR and prints the pin and the whole `PODR` it read
  back from `PCNTR1`,
- prints the S1/S2 levels (released = 1 on the pull-up), then heartbeats.

**Console baud.** Zephyr's `ek_ra6m5` console runs at 115200 on a PLL-derived
PCLKA. 115200 cannot be hit accurately from the 8-MHz MOCO without the SCI
`MDDR` modulator, so this smoke uses **9600 8N1**. Set the J-Link OB virtual
COM port to 9600 on silicon.

## Build and run

```bash
# from the repository root
cargo build -p firmware-ek-ra6m5-smoke --release --target thumbv8m.main-none-eabi

./target/release/labwired test \
  --script examples/ek-ra6m5/io-smoke.yaml \
  --output-dir out/ek-ra6m5-smoke --no-uart-stdout
```

Interactive run (UART to stdout):

```bash
./target/release/labwired \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-ek-ra6m5-smoke \
  --system configs/systems/ek-ra6m5.yaml \
  --max-steps 400000
```

Expected output (first beats):

```text
EK-RA6M5 SMOKE
cpu=cortex-m33 uart=sci0
clk src=moco pclka=8000000 brr=25 baud=9600
led=blue on pin=1 podr=0040
led=blue off pin=0 podr=0000
led=green on pin=1 podr=0080
led=green off pin=0 podr=0000
led=red on pin=1 podr=0100
led=red off pin=0 podr=0000
s1=1 s2=1
hb=0 podr=0040 s1=1 s2=1
...
```

## Board facts used

| Item | Value | Source |
| :--- | :--- | :--- |
| MCU | R7FA6M5BH3CFC (2 MiB code flash, 512 KiB SRAM, 8 KiB data flash) | Zephyr `ek_ra6m5.dts` includes `r7fa6m5bh3cfc.dtsi`; `r7fa6m5xh.dtsi` |
| Core / reset clock | Cortex-M33; MOCO 8 MHz at reset (`SCKSCR` 0x01), every divider /4 (`SCKDIVCR` 0x22022222) | FSP `ra6m5/bsp_feature.h` (`BSP_FEATURE_CGC_STARTUP_SCKSCR`/`_SCKDIVCR`); Zephyr `r7fa6m5xh.dtsi` (`clock-moco` 8 MHz) |
| Flash / RAM | 2 MiB @ 0x00000000 / 512 KiB @ 0x20000000 | Zephyr `r7fa6m5bh3cfc.dtsi`, `r7fa6m5xh.dtsi`; Renode `renesas-r7fa6m5b.repl` |
| LEDs | LED1 blue P006, LED2 green P007, LED3 red P008, active high | Zephyr `ek_ra6m5.dts` |
| Switches | S1 P005, S2 P004, active low, pull-up | Zephyr `ek_ra6m5.dts` |
| Console UART | SCI0 P411 (TX) / P410 (RX) | Zephyr `ek_ra6m5-pinctrl.dtsi`, `zephyr,console = &uart0` |
| SCI0 clock / gate | PCLKA, `MSTPCRB` bit 31 | Zephyr `ra6-cm33-common.dtsi` (`clocks = <&pclka MSTPB 31>`) |

## Explicitly not claimed

- **Clock generation.** The SYSTEM window models only `HOCOCR`/`OSCSF`.
  `SCKDIVCR`, `SCKSCR` and `PRCR` read 0 and drop writes, so the simulator
  neither checks the PRCR key nor applies the divider.
- **Module stop.** `MSTP` is a stub: SCI0 runs whether or not `MSTPB31` is
  cleared.
- **ICU / IELSR.** The ICU is a stub, so no peripheral event reaches the NVIC;
  the smoke polls `SSR.TDRE`.
- **PFS peripheral mux.** `PSEL`/`PMR` are stored only; SCI0 TX does not depend
  on them.
- **TrustZone** (IDAU/SAU, CPSCU/PSCU), flash programming (FACI), the data
  flash and the option-setting memory.
- **DMAC/DTC, ELC, RTC, IWDT/WDT, AGT0-5, GPT, IIC0-2, SPI0/1, ADC12,
  USBFS/USBHS, ETHERC/EDMAC, CANFD, FACI** are declared stubs (reads return 0,
  writes are dropped).
- **SCI1-9, QSPI/OSPI, SDHI, SSIE, CTSU, CEC, DAC12, CRC, DOC** and every other
  block are **not mapped**. Firmware that touches them faults with a memory
  violation.

See `VALIDATION.md` for the tier table.
