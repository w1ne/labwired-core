# SAM E70 Xplained (ATSAME70Q21B) — USART+LED L1 example

Boots a bare-register Cortex-M7 smoke on the Microchip SAM E70 Xplained map
and proves the EDBG console (USART1), the LED0 output path and the SW0 input.

## What the firmware does

1. Disables the watchdog (`WDT_MR.WDDIS`); the WDT runs from reset.
2. Hands `PB4` from JTAG TDI to the PIO (`MATRIX CCFG_SYSIO.SYSIO4`).
3. Gates PIOA/PIOB/PIOC/USART1 (`PMC_PCER0`, ids 10/11/12/14) and waits for
   `PMC_SR.MCKRDY`.
4. Muxes `PA21` to peripheral A (RXD1) and `PB4` to peripheral D (TXD1)
   through `PIO_ABCDSR1/2` and `PIO_PDR`.
5. Resets USART1, sets 8N1 and `BRGR` CD 6 / FP 4 (115384 baud on the 12 MHz
   main RC), then enables TX/RX.
6. Enables the `PA11` pull-up, drives `PC8` and walks LED0 off/on/off,
   printing the pad level and the whole `PIOC_ODSR` each time.
7. Prints a heartbeat that toggles LED0 and reports SW0.

Expected console (asserted by `io-smoke.yaml`):

```text
SAM E70 XPLAINED SMOKE
cpu=cortex-m7 uart=usart1
clk mainck=12000000 cd=6 fp=4 baud=115384
pmc pcsr0=00005c00
mux pa21 psr=0 pb4 psr=0 abcd=11
led=off pin=1 odsr=00000100
led=on pin=0 odsr=00000000
led=off pin=1 odsr=00000100
sw0=1
hb=0 odsr=00000000 sw0=1
hb=1 odsr=00000100 sw0=1
```

## Run it

```bash
# from the repository root
cargo build -p firmware-sam-e70-xplained-smoke --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/sam-e70-xplained/io-smoke.yaml --no-uart-stdout
# stock Zephyr v3.7.0 (committed fixtures; rebuild with
# crates/firmware-sam-e70-xplained-zephyr/build.sh)
cargo run -q -p labwired-cli -- test \
  --script examples/sam-e70-xplained/zephyr-hello.yaml --no-uart-stdout
cargo run -q -p labwired-cli -- test \
  --script examples/sam-e70-xplained/zephyr-sync.yaml --no-uart-stdout
```

`ci/test.sh` runs the same io-smoke, both Zephyr scripts and then the unsupported-instruction audit
with `--fail-on-unsupported`; `configs/ci/boards.yml` runs both on every PR
that touches configs/ or examples/.

## Files

| File | Purpose |
| :--- | :--- |
| `system.yaml` | Board manifest (same as `configs/systems/sam-e70-xplained.yaml`) |
| `io-smoke.yaml` | Ordered UART assertions over 4,000,000 steps |
| `zephyr-hello.yaml` | Stock Zephyr v3.7.0 `hello_world` |
| `zephyr-sync.yaml` | Stock Zephyr v3.7.0 `synchronization` (threads, semaphore, SysTick sleep) |
| `ci/build.sh`, `ci/test.sh` | Board gate build and proof |
| `VALIDATION.md` | Claims, evidence and non-claims |
| `REQUIRED_DOCS.md` | Sources |
| `EXTERNAL_COMPONENTS.md` | Board parts not modelled |

The committed fixture `tests/fixtures/sam-e70-xplained-smoke.elf` is the same
build; `firmware_survival::test_sam_e70_xplained_smoke_survival` boots it.
SIM-DERIVED: there is no SAM E70 bench part.
