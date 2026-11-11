# Validation

The committed onboarding smoke builds a bare-metal RV32IMAC firmware image and runs it on the repository simulator. It checks the UART0 marker, at least eight modeled GPIO10 output edges, and the bounded max-steps stop.

The official ESP32-H2-DevKitM-1 guide identifies the board's USB-to-UART bridge and says available GPIO pins are broken out on the headers. The H2 datasheet assigns UART0 TX to GPIO24 and RX to GPIO23; GPIO10 is available as a general-purpose header pin. This profile captures UART0 bytes in the simulator console and does not simulate the GPIO24/23 IO_MUX route or the physical USB-to-UART bridge.

The UART model covers the FIFO and console register path at the H2 96 MHz core rate. The GPIO model covers the direct output latch and enable path. IO_MUX voltage routing, USB Serial/JTAG, flash boot/XIP, the Thread/Zigbee and BLE radios, and the GPIO8 addressable RGB LED remain outside this profile. Simulator edges do not prove pin voltage or silicon behavior.

Evidence is committed under [evidence/](evidence/). It is local simulator evidence, not a hardware run or silicon correlation.
