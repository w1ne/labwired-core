# Known limitations — ESP32-P4X Function EV Board

This focused profile covers one HP RISC-V execution context, I2C0's command
controller, GPIO8 output registers and matrix selection, and GPIO8's IO_MUX
function-select register. The BME280 is attached directly to the modeled I2C0
controller.

- The board has additional P4 cores and a companion ESP32-C6 module; neither is
  represented in this system.
- The mask ROM, SPI-flash controller and normal board boot flow are not modeled.
  The CLI loads the ELF at its entry point.
- The P4 GPIO matrix does not route the I2C0 controller to physical pads in this
  profile. The I2C transaction and BME280 register response are modeled at the
  controller boundary; SDA/SCL pin waveforms are not.
- USB, Ethernet, audio, camera, display, SD/MMC, PSRAM, and all other GPIO pads
  are outside the profile. The profile does not infer behavior for them.
- The local simulator checks the BME280 ID-gated GPIO transitions. It is not a
  physical-board capture and does not model header loading, LED current,
  voltage, or timing tolerances.
- The LabWired Part Knowledge lookup currently returns `NOT_FOUND` for this
  board; the live vendor and register sources are recorded in the evidence.
