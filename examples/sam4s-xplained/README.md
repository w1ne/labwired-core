# SAM4S Xplained — smoke example

Runs a bare-metal Cortex-M4 firmware on the SAM4S Xplained: the firmware
enables UART0/PIOA/PIOC in the PMC, configures UART0 (8N1, 115200 at 120 MHz),
claims PC10 as a PIO output and PA5 as a pulled-up input, walks the LED with
`PIO_ODSR` read-back, prints the button level and heartbeats.

## Build

```bash
cd core
rustup target add thumbv7em-none-eabi   # once
cargo build -p firmware-sam4s-xplained-smoke --release --target thumbv7em-none-eabi
```

## Run

```bash
cd core
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-sam4s-xplained-smoke \
  --system examples/sam4s-xplained/system.yaml \
  --max-steps 400000
```

Expected console output (prefix):

```
SAM4S XPLAINED SMOKE
cpu=cortex-m4 uart=uart0
led=on odsr=00000400
led=off odsr=00000000
btn=1
hb=0 odsr=00000000 btn=1
...
```

`odsr=` is the PIOC output-data register read from the model (PC10 = bit 10);
`btn=1` is the released, pulled-up button on PA5.

## Smoke test

```bash
cd core
./target/release/labwired test \
  --script examples/sam4s-xplained/io-smoke.yaml \
  --output-dir out/sam4s-xplained-smoke --no-uart-stdout
```

## What the chip models

| Surface | State |
| :--- | :--- |
| ATSAM4S16C CPU (`cortex-m4`, 120 MHz) | modelled |
| Flash (0x00400000, 1 MiB) / SRAM (0x20000000, 128 KiB) | modelled |
| PMC (clock enables, master-clock-ready status) | modelled (`sam4s_pmc`) |
| PIOA/PIOB/PIOC (PER/OER/SODR/CODR/ODSR/PSR/PDSR, pull-ups) | modelled (`sam4s_pio`) |
| UART0/1, USART0/1 (`sam` USART layout) | modelled — UART0 is the console |
| TC0/TC1, SPI0, TWI0/TWI1 | `type: stub` — zeros |
| EEFC, RSTC/SUPC/RTT/WDT/RTC, CHIPID | `type: stub` |
| Silicon capture / executing-fidelity differential | **none** — SIM-DERIVED |

## Documents

- `REQUIRED_DOCS.md` — the sources this package is grounded on
- `EXTERNAL_COMPONENTS.md` — what the board carries beyond the modelled surface
- `VALIDATION.md` — claims, evidence, and explicit non-claims
