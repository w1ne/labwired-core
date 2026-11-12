# Arduino Nano watchdog (`<avr/wdt.h>`)

A stock ArduinoCore-avr sketch that uses the watchdog the way Arduino code does:

1. `.init3` copies `MCUSR`, clears it and calls `wdt_disable()`, as avr-libc recommends.
2. Interrupt mode at 16 ms: `delay(100)` sees six `WDT_vect` hits.
3. `wdt_enable(WDTO_60MS)` with a `wdt_reset()` every 30 ms keeps running for 300 ms.
4. Power-down sleep is woken by a 250 ms watchdog interrupt. Timer0 is stopped in power-down, so `millis()` does not move.
5. `wdt_enable(WDTO_120MS)` followed by a hang restarts the chip, and the next boot reads `WDRF` in `MCUSR`.
6. Interrupt + reset mode: the ISR runs once, its vector entry clears `WDIE`, and the next time-out resets the chip.

The boot counter lives in `.noinit`, which a watchdog reset does not clear.

```bash
pio run -d examples/arduino-nano-wdt
cp examples/arduino-nano-wdt/.pio/build/nano/firmware.elf tests/fixtures/avr/arduino-nano-wdt.elf
```

The proof is `crates/core/tests/avr_nano_wdt.rs`.
