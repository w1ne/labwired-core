# ATmega328PB Arduino fixtures

Built with `build.sh` using PlatformIO `platform = atmelavr`,
`board = ATmega328PB` (MiniCore, variant `atmega328pb`), framework Arduino.

| ELF | Behaviour |
|---|---|
| `atmega328pb-arduino-blink.elf` | `setup()` prints `ATMEGA328PB_ARDUINO_BOOT` on `Serial` (USART0); `loop()` toggles `LED_BUILTIN` (PB5) with `delay(50)` and prints `ATMEGA328PB_ARDUINO_DONE` after three turns. |
| `atmega328pb-arduino-control.elf` | Same banner and the same three 100 ms turns, but no `digitalWrite`; PB5 must stay flat. |

The blink ELF is 14 624 bytes and the control 14 180 bytes. Both include the
stock Arduino core: Timer0 overflow for `millis()`, `delay()`, `HardwareSerial`
on USART0, and the variant pin map.
