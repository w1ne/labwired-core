# CH32V003F4U6 (QFN20)

The QFN20 package of the CH32V003 die. The descriptor
`configs/chips/ch32v003f4u6.yaml` is `ch32v003f4p6` with the QFN20 pin bonding from
WCH DS0 Table 2-1. PC0 and PD5 are bonded, so the committed F4P6 fixtures run
unchanged:

```sh
labwired test --script examples/ch32v003f4u6/io-smoke.yaml    # >= 2 edges on PC0
labwired test --script examples/ch32v003f4u6/uart-smoke.yaml  # "Hello from CH32V003" on USART1
```

To build firmware, see `examples/ch32v003f4p6`. This is local simulator
evidence only. It does not cover WCH-Link, PFIC, ADC, I2C, SPI, timers or USB,
and it is not hardware confirmation.
