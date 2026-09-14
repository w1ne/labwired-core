# Arduino Uno R3 Validation Runbook

Run all commands from `core/`.

## 0) Firmware rationale

Both fixtures are PlatformIO builds of ordinary Arduino sketches for the `uno`
board (`atmelavr`, avr-gcc, Arduino core), committed under
`tests/fixtures/avr/` so the runbook needs no toolchain.

- `arduino-uno-blinky.elf` prints `uno-ok`, then blinks D13 and prints a dot.
- `arduino-uno-io.elf` toggles D7, reads a button on D2 with `INPUT_PULLUP`,
  mirrors it onto D13, and prints `D2=<level> A0=<counts>` every loop.

**What this proves:** the atmega328p descriptor and the Uno manifest boot
Arduino-core firmware from address 0; Timer0 drives `delay`; USART0 TX reaches
the serial capture; each of ports B, C and D reaches its bus-side model in both
directions; and the ADC converts the millivolts a potentiometer drives on A0.

**What it cannot prove:** PWM and Timer1/Timer2, pin interrupts, USART RX,
EEPROM, the watchdog, the Optiboot bootloader, the 16U2 bridge, or silicon
register parity. No hardware capture exists yet.

## 1) Smoke: boot, Timer0, USART0

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-uno-blinky/io-smoke.yaml \
  --no-uart-stdout
```

Pass criteria: exit `0`, `PASS 1/1 checks`, UART contains `uno-ok`.

## 2) Every port and the ADC

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-uno-io/io-smoke.yaml \
  --no-uart-stdout
cargo test -p labwired-core --test avr_uno_io --test avr_uno_machine_run
```

Pass criteria: the smoke prints `D2=1 A0=337` (button released, knob centred);
`avr_uno_io` passes all three tests:

| Test | Asserts |
|------|---------|
| `d7_output_is_visible_on_portd` | D7 is driven and toggles on the `portd` model |
| `a_press_on_d2_reaches_the_sketch_and_lights_d13` | Pressing the D2 button prints `D2=0` and lights D13 |
| `the_potentiometer_on_a0_moves_analog_read` | Knob at 0 %, 50 %, 100 % prints 0, 337, 675 |

## 3) Unsupported-instruction audit

`scripts/unsupported_instruction_audit.sh` covers Thumb and RISC-V only. On an AVR
ELF it exits with `Unsupported architecture: Avr` and executes nothing, so it is
not evidence here. The AVR interpreter returns a decode error on any opcode it
does not implement (`cpu::avr::tests::unknown_opcode_decode_error`), and both
smokes run 2,000,000 instructions to the step limit without one.

## Bench

Flash the committed HEX through the Optiboot bootloader over the USB-B cable,
then open a serial monitor at 9600 baud:

```bash
avrdude -c arduino -p m328p -b 115200 -P <port> \
  -U flash:w:tests/fixtures/avr/arduino-uno-blinky.hex
```

Expected: `uno-ok`, then dots, with LED `L` flickering. A register capture over
ICSP is the next validation step and is not part of this record.

## Validation record

- 2026-09-14: `arduino-uno-blinky` smoke exit 0, `PASS 1/1 checks` over
  2,000,000 steps. A negative control expecting a string the sketch never prints
  fails with 95 UART bytes captured, so the assertion is live.
- 2026-09-14: `arduino-uno-io` smoke exit 0 (`D2=1 A0=337`). `avr_uno_io` 3/3 pass.
  Negative controls: mirroring only PORTB fails the D7 test; reverting the ADC to
  a fixed 512 fails the potentiometer test; composing only PINB from the bus
  fails the D2 test.
- No silicon capture. The board stays datasheet-derived until one exists.
