# Known limitations

This is an L1 board slice. The smoke uses the H2 RISC-V HP core, flash/RAM
address ranges, the GPIO8 output latch, and the board's modeled WS2812 LED.
The H2 GPIO alias reuses the existing Espressif GPIO implementation for the
register offsets exercised here; no H2 GPIO matrix or interrupt behavior is
claimed.

| Area | Covered | Outside this profile |
|---|---|---|
| CPU | Direct-entry RV32IMC firmware execution | Mask ROM startup, bootloader, interrupts, low-power core |
| Memory | 4 MiB flash XIP window and 320 KiB HP SRAM | MMU/cache setup, flash controller, LP SRAM |
| GPIO | GPIO8 output enable and set/clear path, observed pin edges | Other pins, GPIO matrix routing, interrupt behavior, analog levels |
| RGB LED | One GPIO8 WS2812 pixel decodes a red GRB frame | RMT, electrical/power tolerances, color calibration |
| Connectivity | None | UART bridge, USB, Bluetooth LE, 802.15.4, Thread, Zigbee |

The committed evidence is produced by the repository's Core CLI smoke runner.
It is not a hosted LabWired verifier verdict or physical hardware confirmation.
