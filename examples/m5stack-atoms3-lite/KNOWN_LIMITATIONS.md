# Known limitations — M5Stack AtomS3-Lite

The profile covers one behavior: firmware toggles GPIO35 and the attached
WS2812C-2020 model decodes a red pixel. The simulator checks digital pulse
timing and protocol decoding; it does not model electrical LED behavior or
light output.

- The onboard button on GPIO41 is not attached or tested.
- The infrared transmitter on GPIO4 is outside the profile.
- USB download/serial, UART pins, Wi-Fi, Bluetooth, and the board's power
  circuitry are outside the profile.
- Exposed GPIO connector pins are mapped as GPIO only; no sensor or bus device
  is attached to them.
- RMT TX is not included in this profile or smoke.
- The fast-boot local smoke enters the ELF directly and does not exercise the
  ROM, flash bootloader, USB download mode, or an Arduino runtime.
- No physical AtomS3-Lite capture is included.
