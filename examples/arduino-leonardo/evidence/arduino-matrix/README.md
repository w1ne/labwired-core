# Arduino Leonardo — stock ArduinoCore-avr matrix row

Stock ArduinoCore-avr (PlatformIO `atmelavr`, framework-arduino-avr 5.4.0,
board `leonardo`), run by
`validation/arduino-matrix/run_matrix.py --boards arduino-leonardo` on
`configs/chips/atmega32u4.yaml` (system
`validation/arduino-matrix/systems/arduino-leonardo.yaml`).

Board: Arduino Leonardo (A000057), ATmega32U4 @ 16 MHz.

* Chip: microchip.com, read live 2026-11-09 in a real Chrome
  (`mc-live-status.py ATmega32U4`): **In Production**.
* Board: DigiKey search for `A000057`, read 2026-11-09: "ARDUINO LEONARDO W/
  HDRS ATMEGA3", exact match, $23.80. (store.arduino.cc answered HTTP 429 to
  the fetch that day, so the Arduino store itself was not read.)

**8 pass / 1 skip / 0 fail.** L8 is skipped because the ATmega32U4 has no CAN
controller.

On the Leonardo, `Serial` is **native USB CDC**, not a UART. Every cell's
console output went through the twin's USB device controller: the stock
`USBCore.cpp`/`CDC.cpp` attach, bus reset, enumeration by a scripted host, the
CDC port open (SET_LINE_CODING 115200 8N1, SET_CONTROL_LINE_STATE DTR+RTS),
then bulk-IN banks released on SOF. All of these run in the stock core's own
interrupt handlers.

| cell | result | steps | what it proves |
|---|---|---|---|
| L0 serial boot | pass | 424,074 | `USBDevice.attach()` → PLL lock → EORSTI → enumeration → DTR → `Serial.println` over CDC bulk IN (563k cycles = 35 ms) |
| L1 serial loop | pass | 440,168 | `loop()` + `delay()` on Timer0 |
| L2 blink | pass, 3 edges | 452,430 | `digitalWrite(LED_BUILTIN)` toggles D13 = PC7 (`led_watch: portc:7`) |
| L3 I2C | pass, exact tier | 465,096 | `Wire` (TWI, D2/D3) reads INA219 config 0x399F / bus 0x19CA. `uart-full-run.log`: a run to 1.5M steps prints no `LW_L3_PARTIAL_NO_RX` |
| L4 SPI | pass | 439,973 | `SPI` reads the MAX31855 frame 0x01901600 exactly |
| L5 ADC | pass | 426,072 | `analogRead(A0)` = ADC7 (PF7; MUX5 in ADCSRB bit 5) converts the undriven 2.5 V to **512 / 512** (exact AVR tier) |
| L6 PWM | pass, pad edges | 796,935 | `analogWrite(9, 128)` = OC1A on PB5, Timer1 phase-correct: the pad toggles every ~16,380 cycles (≈ 490 Hz, 50 % duty; `pwm_watch: portb:5`) |
| L7 timer | pass | 516,034 | `micros()` advances monotonically across `delay()` |
| L8 CAN | skip | — | no CAN controller on the ATmega32U4 |

Each cell directory holds the matrix's `test.yaml`, `result.json`, `uart.log`
and `junit.xml`. The run stops at the first assertion pass, so `uart.log` is cut
there. Paths are repo-relative, and the firmware is the matrix build of the
cell's sketch.

## Sketch changes for this row

* Every sketch's `while (!Serial) delay(1);` (until now RP2040-only) now also
  applies on `__AVR_ATmega32U4__`. Stock Leonardo CDC drops writes until the
  host raises DTR, on silicon too, so a sketch that prints right after
  `Serial.begin()` loses the line on a real board. The control below shows
  this.
* L6 uses D9 (OC1A). LED_BUILTIN D13 is OC4A on the high-speed Timer4, which
  is not modelled.

## Engine changes this row needed

* `AvrModel::Atmega32u4`: 32 KB flash, SRAM 0x0100..0x0AFF, ports B..F (PIN
  0x23/26/29/2C/2F), the DS7766 Table 9-1 vector table (`m32u4_vector`:
  USB_GEN 11, USB_COM 12, TIMER0_OVF 24, TWI 37, ...), and ADC MUX5 at
  ADCSRB bit 5. The ADC input window moves to 0x1_0200 because PINF would
  overlap the 328P window. OC pins: OC0A PB7, OC0B PD0, OC1A PB5, OC1B PB6.
  USART1 sits at the 328PB USART1 addresses. 0xC0..0xC7 are Timer4 (stored
  only) instead of USART0, and INTn/PCINT are register-only, as on the mega.
* `cpu/avr/usb.rs`: the USB device controller (UHWCON..UEINT, PLLCSR) and the
  host on the cable. The host reuses `peripherals/usb_host.rs`'s enumeration
  script, runs one transfer per 1 ms frame, and opens the CDC ACM interface
  that the configuration descriptor announces. USB_GEN and USB_COM are
  level-sensitive. Frame length follows PLLCSR.PINDIV (16 MHz → 16,000
  cycles).

## Negative control (`controls/no_dtr_wait`)

| control | change | result |
|---|---|---|
| `no_dtr_wait` | L0 without `while (!Serial)` (`main.ino` here) | **fail**: 0 UART bytes. `Serial.println` runs before the host has set DTR, and `Serial_::write` drops it. The console only flows through the full enumerate → line coding → DTR path |

Before the model existed, the same row's L0 reported `unmodeled`.

## Known gaps (not claimed)

* Timer3, the 10-bit high-speed Timer4 (D13/D6 PWM), OC1C, INTn/PCINT on the
  32U4 pins, EEPROM: registers stored only.
* USB: no suspend/resume or remote wake-up, no bulk OUT from the host (so
  `Serial.read()` sees nothing yet), no NAK/error flags. A released IN bank is
  drained at once, so double-bank ping-pong timing is not modelled.
