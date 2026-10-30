# MIMXRT595-EVK (MIMXRT595S) — FLEXCOMM USART+LED L1 example

Boots a bare-register Cortex-M33 smoke on the NXP MIMXRT595-EVK map and
proves the FLEXCOMM0 USART console, the high-speed GPIO output path on the
RGB LED and the SW1/SW2 inputs.

## What the firmware does

1. Carries a FlexSPI configuration block header (tag `FCFB`, version 1.4.0)
   at `0x0800_0400`; `cortex-m-rt` puts the vector table at `0x0800_1000`.
2. Selects the 48 MHz main clock: `CLKCTL0.FRODIVSEL` = 1 (FRO / 4),
   `MAINCLKSELA` = 1 (FRO DIV), `MAINCLKSELB` = 0.
3. Opens the FLEXCOMM0 and HSGPIO0/1/3 clocks through `CLKCTL1.PSCCTL0/1_SET`
   and releases their resets through `RSTCTL1.PRSTCTL0/1_CLR`, then selects
   FRO_DIV4 as the FLEXCOMM0 function clock (`FLEXCOMM[0].FCFCLKSEL` = 0).
4. Muxes P0_1/P0_2 to function 1 (FLEXCOMM0 TXD/RXD), the LED pads to GPIO
   and the switch pads to GPIO with the input buffer on (`IOPCTL`).
5. Selects USART on FLEXCOMM0 (`PSELID` = 1), sets 8N1, `OSR` 16x and
   `BRG` 26 (48 MHz / (16 × 26) = 115384 baud), enables both FIFOs and the
   USART, and prints through `FIFOWR`, polled on `FIFOSTAT.TXNOTFULL`.
6. Makes the three LED pins outputs (`DIRSET`) and walks them: red on through
   `SET[0]`, green through the byte pin `B[1][0]`, blue through the word pin
   `W[3][17]`, red off through `NOT[0]`, red on again through `MPIN[0]` with
   every other bit masked, then all off through `CLR`. Each step reads the
   three pins and the whole of `PIN[0]` back.
7. Reads SW1 (P0_25) and SW2 (P0_10) as inputs (`DIRCLR`).
8. Prints a heartbeat that toggles the green LED through `NOT[1]`.

Expected console (asserted by `io-smoke.yaml`):

```text
MIMXRT595 EVK SMOKE
cpu=cortex-m33 uart=flexcomm0 pselid=000000f1 fifosize=16
fcb tag=42464346 vtor=08001000
clk frodivsel=1 mainclksela=1 pscctl0=00000100 pscctl1=0000000b
uart fclk=48000000 osr=16 brg=26 baud=115384
leds=off r=0 g=0 b=0 pin0=02000400
set red r=1 g=0 b=0 pin0=02004400
byte green r=1 g=1 b=0 pin0=02004400
word blue r=1 g=1 b=1 pin0=02004400
not red r=0 g=1 b=1 pin0=02000400
mpin red r=1 g=1 b=1 pin0=02004400
clr all r=0 g=0 b=0 pin0=02000400
sw1=1 sw2=1
hb=0 green=1
hb=1 green=0
```

`pin0=02000400` is SW1 (bit 25) and SW2 (bit 10) held high by the released
board buttons; bit 14 is the red LED.

## Run it

```bash
# from the repository root
cargo build -p firmware-mimxrt595-evk-smoke --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/mimxrt595-evk/io-smoke.yaml --no-uart-stdout
```

`ci/test.sh` runs the same io-smoke and then the unsupported-instruction audit
with `--fail-on-unsupported`; `configs/ci/boards.yml` runs both on every PR
that touches configs/ or examples/.

## Files

| File | Purpose |
| :--- | :--- |
| `system.yaml` | Board manifest (same as `configs/systems/mimxrt595-evk.yaml`) |
| `io-smoke.yaml` | Ordered UART assertions over 4,000,000 steps |
| `ci/build.sh`, `ci/test.sh` | Board gate build and proof |
| `VALIDATION.md` | Claims, evidence and non-claims |
| `REQUIRED_DOCS.md` | Sources |
| `EXTERNAL_COMPONENTS.md` | Board parts not modelled |

The committed fixture `tests/fixtures/mimxrt595-evk-smoke.elf` is the same
build; `firmware_survival::test_mimxrt595_evk_smoke_survival` boots it.
SIM-DERIVED: there is no MIMXRT595 bench part.
