# ATSAMD21J17D-AFT attempt history

- `2026-11-08-before-uart-only-smoke/` — the measurement before Round 377: the
  chip claimed PM/SYSCTRL/GCLK/NVMCTRL, PORTA and SERCOM0; the smoke was
  100,000 instructions with `uart_contains: "samd21 sercom0 up"` only.
- `2026-11-08-tc-bus-key-config-refusal/` — first run with the new `samd21_tc`
  windows. It failed at chip load: a top-level `bus: sysbus` key flattens into
  `config`, and the TC factory admits no config keys. The two TC entries
  dropped the key; the factory check was kept.
- `2026-11-08-negative-control-no-tc/` — the final firmware against a copy of
  the chip with `tc4`/`tc6` removed (copy included): 2/5 assertions, a precise
  BusFault at `0x42003000` (TC4 CTRLA). The TC checks depend on the model.
- `2026-11-08-negative-control-no-rtc/` — Round 379's firmware (RTC MODE2
  alarm, MODE0 clear-on-match, MODE1 period) against a copy of the chip with
  the new `rtc` window removed (copy included): 2/5 assertions, a precise
  BusFault at `0x40001400` (RTC CTRL). The RTC checks depend on the model.
