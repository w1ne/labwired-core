# External Components (micro:bit v2)

No required external simulated components for the minimal deterministic smoke.

The onboarding path uses on-chip peripherals only:

1. UARTE0 (`uart0`) — EasyDMA console
2. GPIO P0/P1
3. CLOCK (HFCLK/LFCLK behavioural model)
4. SysTick (core)

## On-board devices intentionally omitted

The micro:bit v2 carrier includes:

- a 5x5 row/column-multiplexed LED matrix — attached to GPIO/GPIOTE pad
  levels in both system manifests, with duty-integrated grayscale readback;
- a nRF52833 radio (BLE / 802.15.4) — the RADIO window exists but BLE is not
  modelled;
- an analog MEMS microphone + speaker, a combined motion sensor, a
  touch logo, and the KL27/DAPLink interface MCU — none are attached.

Those are **not** required for the UART smoke path.

## Adding an external device (I²C / SPI sensor, EEPROM, etc.)

See [`examples/demo-blinky/`](../demo-blinky/README.md) — that example is the
canonical reference for the `external_devices` attach pattern. The same
`connection:` / `type:` / `config:` shape works on any chip whose corresponding
bus is modeled.

Before copying, check that the bus you need is actually modeled for nRF52833.
At time of writing the descriptor covers the shared nRF52 UARTE/TWIM/SPIM/
TIMER/RTC/GPIOTE/SAADC/PWM/PPI blocks; the radio, USB and NFC are register
windows without full protocol models.
