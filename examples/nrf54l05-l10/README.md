# nRF54L05 and nRF54L10: application-core siblings

This is an *executable subset*, not a complete Nordic nRF54L05/L10 SoC.
It reuses the nRF54L15 application-core execution path and common peripheral
layouts (UARTE20 EasyDMA, GPIO0/1/2, TIMER10 and SysTick).

The RRAM and SRAM geometries come from Nordic's Zephyr devicetree:

| Part | RRAM at 0 | SRAM at 0x20000000 |
|---|---:|---:|
| nRF54L05 | 500 KiB | 96 KiB |
| nRF54L10 | 1012 KiB | 192 KiB |

The test LED P2.09 and UARTE20 wiring are *synthetic*: this is not a claim
that a specific development kit has the same LED or VCP pins.
The bare-metal firmware uses nRF54L-family UARTE EasyDMA and has no
LabWired-specific shim. ARM Cortex-M33 startup is reused from nRF54L15
but linked with each sibling's own memory map.

## Run tests

From the labwired-core repository root:

```sh
./examples/nrf54l05-l10/firmware/build.sh
labwired test --script examples/nrf54l05-l10/nrf54l05-smoke.yaml \
  --output-dir out/nrf54l05 --no-uart-stdout
labwired test --script examples/nrf54l05-l10/nrf54l10-smoke.yaml \
  --output-dir out/nrf54l10 --no-uart-stdout
```

Each should pass 4/4 assertions: UART chip identification, memory geometry
banner, observable GPIO edges, and expected stop reason.

Not modeled or verified: Bluetooth/radio, TrustZone secure boot, crypto,
other serial personalities, full clock tree, or silicon parity.
A production board requires a separate devicetree, pin mapping and hardware
capture before any claim of full board support.
