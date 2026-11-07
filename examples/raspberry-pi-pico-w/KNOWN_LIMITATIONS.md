# Known limitations

This onboarding profile covers only the RP2040 GP0 header pin: the firmware
selects SIO in IO_BANK0 and toggles the output through SIO. The smoke asserts
at least eight transitions on `sio:0`.

Pico W's CYW43439 wireless module is not present in the system descriptor. The
profile does not model or claim Wi-Fi, Bluetooth, the module's firmware or bus,
or the onboard LED attached to that module. USB CDC, UF2 boot, USB, and all
other RP2040 peripherals are outside this smoke.

The board uses 2 MiB QSPI flash and 264 KiB SRAM in this descriptor, but the
firmware runner directly maps the application image after the reserved
second-stage bootloader slot. It does not simulate physical USB flashing or
the RP2040 boot ROM. No physical-board capture is included.
