# Teensy 4.1 modeled scope

No external simulated components are needed for this GPIO smoke. The focused
system contains the modeled CCM clock controller and GPIO2 controller used by
the onboard D13 LED path.

Not represented by this profile: IOMUXC alternate-function routing, LPUART6
pin routing, USB, Ethernet/PHY, SD, external QSPI flash/XIP, and optional
PSRAM. The separate LPUART regression fixture is a peripheral test and is not
a Teensy 4.1 serial-board claim.
