# External components and profile scope

The tested profile uses the XIAO's built-in USER_LED on GPIO21 and native USB
Serial/JTAG output. It needs no external sensor or breakout. The S3R8 PSRAM
is physically present but not represented in this profile; the smoke is built
in QSPI-only mode. USB electrical signaling and enumeration,
camera/microphone/SD expansion, battery circuitry, wireless radios, and charge
indicator are outside this smoke.

The ESP32-S3 `system` register window is still declared as the engine's
zero-answering `system_stub`; this GPIO/USB-serial smoke does not access it.
