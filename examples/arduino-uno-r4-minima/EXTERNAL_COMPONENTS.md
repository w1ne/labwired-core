# External Components (Arduino Uno R4 Minima)

The BME280 is attached at I2C address `0x76` on modeled IIC1 for the sensor
smoke. It is an external component, not an on-board part.

The onboarding path uses on-chip peripherals only:
1. SYSTEM (`ra_sysc` — HOCO / OSCSF)
2. PORT1 / PORT3 (`ra_port`)
3. SCI2 (USART / header Serial)
4. IIC1 (`ra_iic`, A4/SDA=P101 and A5/SCL=P100 header mapping)
5. SysTick

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
The RA4M1 IIC1 master is modeled for register-level transactions with attached
I2C devices. PFS values can be written, but the simulator does not electrically
route IIC1 through P100/P101; that pad behavior is outside this smoke's claim.
SPI, GPT, ICU event linking, MSTP, and USB CDC remain outside this board
profile's exercised behavior.
