# External Components (Arduino Uno R3)

No external simulated component is required for the smoke. The LED on D13 is a
`board_io` entry in `configs/systems/arduino-uno.yaml`.

## Onboard parts

| Part | Where | Twin |
|------|-------|------|
| LED `L` | D13 / PB5 | `board_io` LED `status_led` on `portb` pin 5 |
| LEDs `TX`, `RX` | Driven by the 16U2 | Not modeled |
| LED `ON` | 5 V rail | Not modeled |
| ATmega16U2 | USB-B to USART0 bridge | Not modeled as a chip. USART0 TX is the serial pane. |
| 16 MHz crystal / resonator | Clocks for both processors | Clock is `cpu_hz: 16000000` in the manifest |
| SPX1117M3-L-5 regulator | DC jack to 5 V | Power is digital in the twin |
| LMV358 op-amp | USB/VIN power selection, LED buffer | Not modeled |
| Reset button | RESET / PC6 | Not a GPIO. Use the run controls to restart. |
| ICSP headers (×2) | 328P and 16U2 programming | Not modeled. Load the ELF directly. |

## Adding an external device

Parts attach through `external_devices` in a system.yaml, as in
[`examples/arduino-uno-io/system.yaml`](../arduino-uno-io/system.yaml):

- GPIO parts (LED, button) use `board_io` on `portb` (D8-D13), `portc` (A0-A5 as
  digital) or `portd` (D0-D7).
- Analog parts (potentiometer, NTC) use `connection: "adc"` with `channel` 0-5 for
  A0-A5.
- SPI parts attach to `spi` (D10-D13); I2C parts attach to `i2c` (A4/A5, also on
  the SDA/SCL header pins).

The potentiometer model runs from a 3.3 V rail. On a 5 V board, full travel reads
about 675 counts; wire the real part's top leg to 3.3V to match.
