# ESP32-C3 headless JTAG watch

This is a minimal Arduino ESP32-C3 watch fixture for compile and ROM-boot
validation. It has no display or I2C dependency.

The firmware starts at `12:34:00`, emits UART0 lines such as
`WATCH 12:34:00 RUN`, and advances with a wrap-safe `millis()` elapsed-time
gate. GPIO4 (`MODE_PIN`) toggles run/setting mode; GPIO5 (`SET_PIN`) advances
the minute and resets seconds while setting. Both inputs use `INPUT_PULLUP` and
active-low falling-edge debounce.

UART0 is the fixture's only simulator display.

The bundled system manifest provides the two active-low simulation inputs; it
does not assert any physical button wiring.

Open it in Studio at
[`https://app.labwired.com/?board=esp32c3-jtag-watch`](https://app.labwired.com/?board=esp32c3-jtag-watch).

`labwired_watch_state` is retained for debugger/JTAG inspection. Its packed
layout is:

- bits 0..5: seconds
- bits 6..11: minutes
- bits 12..16: hours
- bit 17: setting mode
- bits 18..31: monotonically increasing 14-bit state sequence

`labwired_watch_state_schema` contains `0x57415431` (`WAT1`).

Build locally with:

```sh
pio run
```
