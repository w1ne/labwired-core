# ATmega328PB USART1 smoke

This example targets the buyable ATmega328PB-AU profile. It configures the
second USART, sends `PB_USART1_OK` through UDR1, and holds PB5 high. The model
routes USART1 polling TX/RX registers through a distinct host window; USART1
interrupts are not modelled yet.

Build and run the checked-in AVR-GCC firmware image from the core repository:

```bash
examples/atmega328pb-usart1/ci/build.sh
examples/atmega328pb-usart1/ci/test.sh
```

The local AVR-GCC 7.3 package knows the `atmega328pb` code-generation target,
but its AVR-LibC package lacks the PB register header and startup object. This
example therefore defines the data-space register addresses from Microchip's
[ATmega328PB data sheet](https://ww1.microchip.com/downloads/aemDocuments/documents/MCU08/ProductDocuments/DataSheets/40001906C.pdf)
and provides a small reset vector. The test boots that compiled ELF in the core
simulator and checks USART1 output plus PB5 high; it is not a physical-board or
silicon-capture result. USART1 interrupt behavior is not modelled yet.
