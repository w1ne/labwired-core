# Validation — Seeed Studio XIAO RP2040

The committed smoke builds bare-metal RP2040 firmware and runs it against the
repository's register-level model. It asserts at least eight transitions on
each of the three active-low LED GPIOs and stops at the configured instruction
limit.

The board and LED mappings come from Seeed's [XIAO RP2040 hardware guide](https://wiki.seeedstudio.com/XIAO-RP2040/),
which identifies the red, green and blue LEDs on GPIO17, GPIO16 and GPIO25 and
documents their active-low behavior. Seeed's guide lists the RP2040, 2 MB
onboard flash and 264 KB SRAM for this board. The RP2040 model is limited here
to the real IO_BANK0 and SIO implementations used by the firmware.

The CLI build log, firmware build log, smoke log and simulation artifacts are
committed under [`evidence/`](evidence/). This is model evidence from the
repository engine, not a physical-board capture.
