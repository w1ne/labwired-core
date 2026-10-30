# FRDM-K64F — LabWired example

The [FRDM-K64F](https://www.nxp.com/design/design-center/development-boards-and-designs/FRDM-K64F)
board with an **MK64FN1M0VLL12**: Cortex-M4F, 1 MiB flash, 256 KiB SRAM
(64 KiB SRAM_L + 192 KiB SRAM_U), 120 MHz max. This is the **UART-only L1
slice** — the Ethernet/TCP milestone is explicitly deferred.

## What runs here

A bare-register smoke firmware (`crates/firmware-frdm-k64f-smoke`) that:

- enables the SIM gates (`SCGC4` UART0, `SCGC5` PORTB/PORTE),
- muxes **PTB16/PTB17** to **UART0 ALT3** and the LED pins to GPIO,
- walks the active-low **RGB LED** — red PTB22, green PTE26, blue PTB21 —
  and prints the **PDOR bit it actually drove**,
- heartbeats on **UART0** at 115200 8N1 from the reset FEI clock
  (20.97152 MHz).

## Build and run

```bash
# from the repository root
cargo build -p firmware-frdm-k64f-smoke --release --target thumbv7em-none-eabi

./target/release/labwired test \
  --script examples/frdm-k64f/io-smoke.yaml \
  --output-dir out/frdm-k64f-smoke --no-uart-stdout
```

Interactive run (UART to stdout):

```bash
./target/release/labwired \
  --firmware target/thumbv7em-none-eabi/release/firmware-frdm-k64f-smoke \
  --system configs/systems/frdm-k64f.yaml \
  --max-steps 400000
```

Expected output (first beats):

```text
FRDM-K64F SMOKE
cpu=cortex-m4 uart=uart0
led=red on pin=0
led=red off pin=1
led=green on pin=0
led=green off pin=1
led=blue on pin=0
led=blue off pin=1
hb=0 red=0 green=1 blue=1
...
```

## Board facts used

| Item | Value | Source |
| :--- | :--- | :--- |
| MCU | MK64FN1M0VLL12, LQFP100 | NXP FRDM-K64F user manual; Zephyr `frdm_k64f` |
| Core / clock | Cortex-M4F, 120 MHz max; 20.97152 MHz FEI at reset | K64 RM §5; datasheet |
| Flash / RAM | 1 MiB @ 0x00000000 / 64 KiB @ 0x1FFF0000 + 192 KiB @ 0x20000000 | K64 RM §3; Zephyr `nxp_k6x.dtsi` |
| RGB LED | red PTB22, green PTE26, blue PTB21, active low | FRDM-K64F user manual; Zephyr dts |
| Buttons | SW2 PTC6, SW3 PTA4, active low | FRDM-K64F user manual; Zephyr dts |
| Console UART | UART0 PTB16 (RX) / PTB17 (TX), ALT3, 115200 8N1 | Zephyr `frdm_k64f` pinctrl |

## Explicitly not claimed

- **WDOG** is not modelled (a stub: no timeout, no reset). The firmware
  unlocks and disables it at the top of `main`, as it must on silicon, where
  WDOG is enabled out of reset. The ELF also carries the unsecured Flash
  Configuration Field at `0x400` (FSEC `0xFE`), so it is safe to flash.
- **SIM clock gating** is not modelled (the SIM window is a stub). The firmware
  writes `SCGC4`/`SCGC5` as silicon requires, but the simulator does not gate on
  them.
- **PORT pad muxing/pulls** are not modelled (PORTx PCR is a stub). The GPIO
  output path and the LED readback are real; UART0 pin muxing is not enforced.
- **MCG PLL/FEE** is not modelled. The smoke runs on the reset FEI clock; the
  K64F PLL (C5/C6, `S_LOCK0`/`PLLST`) is the Zephyr `hello_world`/TCP schedule
  risk.
- **Ethernet MAC + KSZ8081 PHY**, RNGA and FTFE are declared stubs (reads
  return 0, writes are dropped). The Zephyr TCP test is deferred.
- **eDMA, PIT/FTM, I2C/SPI, ADC/DAC, USB, CAN and SDHC are not mapped.**
  Firmware that touches them faults with a memory violation.

See `VALIDATION.md` for the tier table.
