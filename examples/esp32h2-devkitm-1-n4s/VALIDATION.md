# Validation

The committed onboarding smoke builds a bare-metal RV32IMAC firmware image and
runs it on the repository simulator. It reads BME280 chip ID 0x60 and the
modeled raw temperature over I2C0 on GPIO4/GPIO5, checks the UART0 marker, sees
at least eight GPIO10 output edges, and stops at the configured step bound.

The official ESP32-H2-DevKitM-1 guide lists GPIO4 and GPIO5 on J1 pins 9 and
10 and GPIO10 on J3 pin 4. The smoke configures I2CEXT0 matrix signals 45/46 on
GPIO4/GPIO5. The I2C model reuses the shared Espressif command-list engine,
paced at the H2 96 MHz core rate; the H2 GPIO model recognizes only the
I2C0-relevant matrix selectors plus direct output. The BME280 is an attached
device in the system manifest, not an onboard component.

The UART model captures the FIFO console path; it does not model the GPIO24/23
IO_MUX route or physical USB-to-UART bridge. IO_MUX electrical behavior, clock
tree, I2C1, USB Serial/JTAG, flash boot/XIP, Thread/Zigbee and BLE radios, and
the GPIO8 addressable RGB LED remain outside this profile. The result is local
simulator evidence, not hardware or silicon confirmation.

Evidence is committed under [evidence/](evidence/).
