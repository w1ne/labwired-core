# XIAO RP2350 validation

On 2026-11-03, the repository onboarding smoke built the CLI and the
`thumbv8m.main-none-eabi` firmware, then ran the binary with
`configs/systems/seeed-xiao-rp2350.yaml`.

Result: **pass, 2/2 checks**. The SIO GPIO trace recorded at least eight edges
on GP25 (`sio:25`), then the run stopped at the configured 200,000-step limit.
The captured output is in [`evidence/`](evidence/), including the build and
smoke logs, metrics, JUnit report, result, and final snapshot.

This is a local deterministic engine smoke for the narrow board profile. It
does not claim physical hardware confirmation. The profile contains only the
RP2350 memory, IO_BANK0 pad mux, SIO GPIO, and the active-low USER LED. RGB LED
PIO/WS2812 signaling, USB, battery measurement, other header GPIO, and the
second M33/Hazard3 execution paths remain outside its scope.

The broader `configs/chips/rp2350.yaml` still contains zero-response `POWMAN`
and `TBMAN` windows. Neither is loaded by this narrow profile or accessed by
the smoke firmware.
