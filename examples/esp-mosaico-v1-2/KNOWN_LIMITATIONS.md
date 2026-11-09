# Known limitations — ESP-Mosaico V1.2

This focused profile covers one HP RISC-V execution context, GPIO7's input
buffer and pull-up, GPIO8's output latch and matrix selection, their IO_MUX
registers, and the board's function-button stimulus.

- The second HP core, LP CPU, ROM, flash controller and normal boot sequence are
  not modeled. LabWired loads the firmware ELF directly into internal HP SRAM.
- Only GPIO7 and GPIO8 are represented. Other board pins and integrated devices
  (radio, USB, displays, sensors and expansion modules) are outside the profile.
- GPIO8 controls the board's vibration motor. The simulator checks the electrical
  control-pin state and transitions; it does not model motor current or vibration.
- The button is driven by deterministic simulator input. No physical board run
  or hardware capture is claimed.
