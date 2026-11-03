# External components — nRF52 DK

The onboarding system has no attached external devices. The smoke uses the
on-board LEDs, the nRF52832 GPIO block, and the UARTE0 transmit path.

The full board also includes an NFC antenna, RF antenna, CR2032 holder, and a
separate J-Link/interface MCU. The interface MCU and its USB/J-Link behavior
are not simulated. The UARTE0 host console abstracts the board's VCOM endpoint;
it does not model the interface firmware. Radio and NFC behavior are outside
the smoke's scope.
