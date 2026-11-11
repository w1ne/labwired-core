# NUCLEO-G431RB

Demand rank: **other STM32, rank 4** (13% in the JetBrains State of Embedded
2025 survey); **Nucleo/Discovery kits, rank 3** (23% of main-project kits).
The STM32G4 line is a measured professional motor-control target. ST currently
lists NUCLEO-G431RB as active. Its standalone store listing is out of stock,
but the in-stock [P-NUCLEO-IHM03 motor-control pack](vendor-status.md) includes
this board.

## Board profile

| Nucleo connection | MCU signal | Model mapping |
| --- | --- | --- |
| LD2 user LED / Arduino D13 | PA5 | `gpioa` bit 5, active high |
| B1 user button | PC13 | `gpioc` bit 13, active low |
| ST-LINK virtual COM (default solder bridges) | LPUART1 TX/RX on PA2/PA3 | `lpuart1` at `0x40008000` |

The board profile uses the existing STM32G431RB chip and G4 peripheral models.
The board-specific firmware smoke requires its `NUCLEO_G431RB_LPUART1_OK`
marker from the LPUART1 TX register model plus three observed PA5 edges. This
checks the user LED output and the UART peripheral. It does not simulate the
PA2/PA3 electrical path through the ST-LINK virtual-COM bridge.

## Run the smoke

From the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id nucleo-g431rb \
  --script examples/nucleo-g431rb/io-smoke.yaml \
  --system configs/systems/nucleo-g431rb.yaml \
  --out-dir examples/nucleo-g431rb/evidence \
  --firmware-build-script examples/nucleo-g431rb/build-firmware.sh
```

Evidence is committed under `evidence/`. The ST-LINK USB bridge, full motor
pack, X-NUCLEO-IHM16M1 shield, STSPIN830, and motor are outside this board
profile. The smoke does not claim motor-control, ADC, analog, or physical-board
behavior.
