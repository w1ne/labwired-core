# Known limitations — ESP32-P4X Function EV Board

This focused profile covers one HP RISC-V execution context, GPIO8 output
registers and matrix routing, and the GPIO8 IO_MUX function-select register.

- The board has additional P4 cores and a companion ESP32-C6 module; neither is
  represented in this system.
- The mask ROM, SPI-flash controller and normal board boot flow are not modeled.
  The CLI loads the ELF at its entry point.
- USB, Ethernet, audio, camera, display, SD/MMC, PSRAM, and all other GPIO pads
  are outside the profile. The profile does not infer behavior for them.
- The local simulator checks GPIO transitions only. It is not a physical-board
  capture and does not model header loading, LED current, voltage, or timing
  tolerances.
- The LabWired Part Knowledge lookup currently returns `NOT_FOUND` for this
  board; the live vendor and register sources are recorded in the evidence.
