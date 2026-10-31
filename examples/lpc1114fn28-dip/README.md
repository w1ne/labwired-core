# LPC1114FN28/102 (DIP-28) — UART+LED L1 example

Boots a bare-register Cortex-M0 smoke on the NXP LPC1114FN28/102 map and
proves NXP's CMSIS clock bring-up, the UART console, the GPIO output path on
the PIO0_7 LED and the GPIO0 edge interrupt.

## What the firmware does

1. Runs CMSIS `SystemInit` with the mbed LPC1114 settings: `SYSPLLCLKSEL` =
   IRC, `SYSPLLCLKUEN` 1/0/1 (polling bit 0), `SYSPLLCTRL` = `0x23` (M = 4,
   P = 2: 12 MHz → 48 MHz), PLL power-up (`PDRUNCFG` bit 7 = 0) and the
   `SYSPLLSTAT.LOCK` wait, `MAINCLKSEL` = PLL out, `MAINCLKUEN` 1/0/1,
   `SYSAHBCLKDIV` = 1, then the IOCON clock.
2. Enables the UART clock, sets `UARTCLKDIV` = 1 and muxes PIO1_6/PIO1_7 to
   RXD/TXD (IOCON `FUNC` = 1).
3. Sets the UART to 8N1 at 115089 baud (mbed's `serial_api.c` search for
   115200 from 48 MHz): `LCR.DLAB`, `DLM` = 0, `DLL` = 17, `FDR` = `0xF8`
   (MULVAL 15, DIVADDVAL 8), `DLAB` clear, FIFOs enabled and reset.
4. Prints `DEVICE_ID`, the clock registers and the UART divisor, read back
   through `DLAB`.
5. Makes PIO0_7 a GPIO output, drives it low and high through its
   address-masked `DATA` window, and checks that a write through another
   pin's mask leaves it alone.
6. Arms a rising-edge interrupt on PIO0_7 (`IEV`, `IE`, NVIC line 31) and
   toggles the pad three times; the handler counts the edges and clears them
   through `IC`.
7. Prints a heartbeat that toggles the LED.

Expected console (asserted by `io-smoke.yaml`):

```text
LPC1114FN28 SMOKE
cpu=cortex-m0 uart=uart pins=pio1_6/pio1_7 led=pio0_7
device_id=1a40902b
clk pllctrl=23 lock=1 mainclksel=3 ahbclkctrl=1185f
uart lcr=03 dl=17 fdr=f8 ter=80 baud=115089
led=off data=f7f led=0 ris=080
mask6 data=f7f led=0 ris=080
led=on data=fff led=1 ris=080
irq gpio0 edges=3 mis=000
hb=0 led=1
hb=1 led=0
```

`ris=080` on the three LED lines is the reset interrupt sense (`IS` = 0,
`IEV` = 0: falling edge). Driving the pulled-up pad low latches it, as it
does on silicon; `IE` is still 0, so no interrupt is taken.

## Run it

```bash
# from the repository root
cargo build -p firmware-lpc1114fn28-smoke --release --target thumbv6m-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/lpc1114fn28-dip/io-smoke.yaml --no-uart-stdout
```

`ci/test.sh` runs the same io-smoke and then the unsupported-instruction audit
with `--fail-on-unsupported`; `configs/ci/boards.yml` runs both on every PR
that touches configs/ or examples/.

## Files

| File | Purpose |
| :--- | :--- |
| `system.yaml` | Board manifest (same as `configs/systems/lpc1114fn28-dip.yaml`) |
| `io-smoke.yaml` | Ordered UART assertions over 2,000,000 steps |
| `ci/build.sh`, `ci/test.sh` | Board gate build and proof |
| `VALIDATION.md` | Claims, evidence and non-claims |
| `REQUIRED_DOCS.md` | Sources |
| `EXTERNAL_COMPONENTS.md` | Breadboard parts not modelled |

The committed fixture `tests/fixtures/lpc1114fn28-smoke.elf` is the same
build; `firmware_survival::test_lpc1114fn28_dip_smoke_survival` boots it.
SIM-DERIVED: there is no LPC1114 bench part.
