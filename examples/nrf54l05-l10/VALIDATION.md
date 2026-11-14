# nRF54L05/L10 executable-family evidence — 2026-11-13

## Sources

- Zephyr Nordic SoC devicetree: `zephyr/dts/vendor/nordic/nrf54l05.dtsi`,
  `nrf54l10.dtsi`, `nrf54l_05_10_15.dtsi`:
  https://github.com/zephyrproject-rtos/zephyr/tree/main/dts/vendor/nordic
- Existing LabWired `configs/chips/nrf54l15.yaml` register subset, sourced
  from Nordic SVD/MDK and Zephyr DTS.
- Nordic product pages: https://www.nordicsemi.com/Products/nRF54L05
  and https://www.nordicsemi.com/Products/nRF54L10.

## Executed local evidence

Built with ARM GCC from checked-in source and linker scripts.
Local `labwired-cli 0.25.0` on work-1, 2026-11-13:

- `nrf54l05-smoke.yaml`: **PASS 4/4**, 100000 steps, 0.28s.
- `nrf54l10-smoke.yaml`: **PASS 4/4**, 100000 steps, 0.02s.
- Assertions: distinct UART chip banner, RRAM+SRAM geometry banner, GPIO P2.09
  edges and expected `max_steps` stop.

Committed fixtures:

- `nrf54l05.elf` SHA256:
  `761584ebd37bd8cc17c629fa0677cf8cd0d340ffae66bf619380b6553df08614`
- `nrf54l10.elf` SHA256:
  `62c457687156d2e562d82391b989b01cb428d6c0a0b7a4ce2b72e1219e522dc6`

Tests use synthetic GPIO wiring and shared models. These are **not**
Nordic SDK/Zephyr boot tests, radio tests or hardware captures. Physical
boards, BLE/radio and TrustZone require separate qualification.

## Negative control

Alter `uart20` base in the L05 descriptor from `0x500C6000` to
`0x500C8000` (only in a temporary copy). Run the same L05 ELF.
Result: **no** `NRF54L05_OK` banner and a precise BusFault at
`0x500C6604` (UART20 PSEL.TXD access). This proves that the successful
test observes the chip register map rather than treating any MMIO as success.
On the locally installed CLI the standalone `run` command returned 0 even
when reporting the BusFault, so do not use its exit code alone as an oracle;
the `test` assertions are the acceptance gate.
