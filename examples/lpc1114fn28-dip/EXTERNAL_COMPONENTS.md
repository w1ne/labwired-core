# External Components — LPC1114FN28/102 (breadboard)

None are modelled for the UART+LED L1 slice. The breadboard circuit the smoke
assumes is:

- an LED with a series resistor from PIO0_7 to GND (active high), listed as
  `board_io` `led1` in the system manifest;
- a 3.3 V USB-UART adapter on PIO1_7 (TXD) / PIO1_6 (RXD). The simulator
  captures the UART stream directly; the adapter is not modelled.

When a later slice attaches a part (for example an I2C sensor on PIO0_4 /
PIO0_5), it goes here with its fidelity tier and descriptor path, and the
system manifest grows an `external_devices:` entry.
