# External Components (Arduino Uno R4 Minima)

No required external simulated components for minimal deterministic smoke.

The onboarding path uses on-chip peripherals only:
1. SYSTEM (`ra_sysc` — HOCO / OSCSF)
2. PORT1 / PORT3 (`ra_port`)
3. SCI2 (USART / header Serial)
4. SysTick

## On-board devices intentionally omitted

The Uno R4 Minima carrier includes USB Full-Speed (USBFS) used by Arduino for
USB CDC Serial. USB CDC is **out of scope** here — `usbfs` is a stub MMIO
window only, and the smoke path uses SCI2 on D0/D1.

## Adding an external device (I²C / SPI sensor, EEPROM, etc.)

See [`examples/demo-blinky/`](../demo-blinky/README.md) — that example is
the canonical reference for the `external_devices` attach pattern (TMP102
on I²C1, STM32F103). The same `connection:` / `type:` / `config:` shape
works on any chip whose corresponding bus is modeled.

Before copying, check that the bus you need is actually modeled for RA4M1.
At time of writing the Uno R4 Minima model covers SYSTEM / PORT / SCI2 UART
/ SysTick only — IIC, SPI, GPT, ICU event linking, MSTP, PFS, and USB CDC
are not yet modeled end-to-end.
