# Apollo4 Plus EVB — LabWired example

The [Ambiq Apollo4 Plus EVB](https://ambiq.com/apollo4-plus/) (ordering code
**AMAP4PEVB**): an **AMAP42KP-KBR** Apollo4 Plus SoC (Cortex-M4F, up to
192 MHz, 2 MiB MRAM, 2.75 MiB RAM) with an on-board SEGGER J-Link. This is the
**UART+LED L1 slice** and the first Ambiq part — the rest of the SoC is
declared stubs.

## What runs here

A bare-register smoke firmware (`crates/firmware-apollo4p-evb-smoke`) that:

- powers the **UART0** domain (`PWRCTRL_DEVPWREN.PWRENUART0`) and polls
  `DEVPWRSTATUS.PWRSTUART0`, as `am_hal_pwrctrl_periph_enable` does,
- unlocks the pads (`GPIO_PADKEY = 0x73`), muxes **GPIO60** to UART0 TX and
  **GPIO47** to UART0 RX (`FNCSEL 4`), makes **LED0 GPIO30, LED1 GPIO90,
  LED2 GPIO97** push-pull outputs (driven high = off first, input buffer on
  for read-back) and **BTN0 GPIO18 / BTN1 GPIO19** inputs on the 24K
  pull-up, then relocks,
- selects the 24-MHz UART clock (`CR.CLKSEL = 1`) and derives the UART0
  divisors from it — `IBRD = 13`, `FBRD = 1` for 115200 8N1,
- reads the core performance mode from `PWRCTRL_MCUPERFREQ` (LP, 96 MHz),
- walks the three LEDs (active low) with single-bit `WTC`/`WTS` strobes and
  prints each pad level read back through `RD` plus the lit-LED mask, checks
  that each bank's `WT` latch holds exactly its LED bit,
- prints the two buttons' pad levels (released = 1 on the pull-up), then
  heartbeats with LED0 lit.

The image is linked at `0x0001_8000`, after the secure bootloader, so the
same ELF is laid out the way the Apollo4 Plus SBL expects. The simulator
fast-boots it at that vector table.

## Build and run

```bash
# from the repository root
cargo build -p firmware-apollo4p-evb-smoke --release --target thumbv7em-none-eabi

./target/release/labwired test \
  --script examples/apollo4p-evb/io-smoke.yaml \
  --output-dir out/apollo4p-evb-smoke --no-uart-stdout
```

Interactive run (UART to stdout):

```bash
./target/release/labwired \
  --firmware target/thumbv7em-none-eabi/release/firmware-apollo4p-evb-smoke \
  --system configs/systems/apollo4p-evb.yaml \
  --max-steps 400000
```

Expected output (first beats):

```text
APOLLO4P EVB SMOKE
cpu=cortex-m4f uart=uart0
pwr perf=lp uart0_pwr=1
uart0 clksel=1 clk=24000000 ibrd=13 fbrd=1
led=led0 on rd=0 lit=1
led=led0 off rd=1 lit=0
led=led1 on rd=0 lit=2
led=led1 off rd=1 lit=0
led=led2 on rd=0 lit=4
led=led2 off rd=1 lit=0
wt0=1 wt2=1 wt3=1
btn0=1 btn1=1
hb=0 led0=0 led1=1 led2=1
hb=1 led0=0 led1=1 led2=1
...
```

## Board facts used

| Item | Value | Source |
| :--- | :--- | :--- |
| MCU | AMAP42KP-KBR (Apollo4 Plus, 146-ball BGA) | Ambiq AMAP4PEVB listing; Zephyr `apollo4p_evb` doc |
| Core / clock | Cortex-M4F, 192 MHz max; boots in LP mode (96 MHz) | `apollo4p.h` MCUPERFREQ enums |
| MRAM | 2 MiB @ 0x0; SBL owns 0x0-0x17FFF; application @ 0x00018000 | `am_reg_base_addresses.h`; Zephyr `ambiq_apollo4p.dtsi` flash0 |
| RAM | TCM 384 KiB @ 0x10000000, then SSRAM0 1 MiB, DSP RAM 384 KiB, SSRAM1 1 MiB (2816 KiB contiguous) | `am_reg_base_addresses.h` |
| LEDs | LED0 GPIO30, LED1 GPIO90, LED2 GPIO97, active low | Zephyr `apollo4p_evb.dts` |
| Buttons | BTN0 GPIO18, BTN1 GPIO19, active low, pull-up | Zephyr `apollo4p_evb.dts` |
| Console UART | UART0 TX GPIO60 / RX GPIO47 (FNCSEL 4), 115200 8N1, J-Link VCOM | Zephyr `apollo4p_evb-pinctrl.dtsi`, `doc/index.rst`; `apollo4p.h` FNCSEL enums |

## Explicitly not claimed

- The **boot ROM and secure bootloader** are not present; the engine enters
  the ELF at its `0x0001_8000` vector table, which is where the SBL hands
  over on silicon.
- **Power domains** have no effect: `DEVPWRSTATUS` follows `DEVPWREN` on the
  same access and a peripheral works with its domain off.
- The **UART clock selection** (`CR.CLKEN`/`CR.CLKSEL`) is written as on
  silicon but not checked: the shared PL011 model does not store `CR`, so the
  firmware derives the divisor from its own `CLKSEL` constant instead of
  reading `CR` back.
- The **pad mux** for non-GPIO functions is not routed: `FNCSEL 4` on
  GPIO60/47 is stored, and the console model does not depend on it.
- **GPIO interrupts** are never raised (software can only set/clear STAT).
- **CLKGEN, MCUCTRL, RSTGEN, IOM0-7, MSPI0-2, ADC, timers, STIMER, RTC, WDT,
  security, crypto, GPU/display, SDIO, USB, audio** are declared stubs.

See `VALIDATION.md` for the tier table.
