# Validation

Demand rank: ESP32 family **#1** in the measured onboarding evidence. The
survey does not distinguish ESP32-P4 as a separate chip; this is a family
depth addition.

The smoke boots a firmware image linked into internal HP SRAM and checks for
GPIO2 pad transitions after programming GPIO2's IO_MUX function, direct-output
selector, output-enable source, latch, and enable bit. The board's LED mapping
is documented, while the LED component itself is not simulated.

The committed evidence under `evidence/` is a local LabWired CLI onboarding
smoke: `status: pass`, 300,000 steps, both assertions passed. The captured
GPIO edge trace and structured verdict are in
`evidence/simulation-output/result.json`; per-stage console logs are in
`evidence/logs/`. It is not a physical board run or a claim of full ESP32-P4 support.
GPIO2 LED output is the only routed pad behavior. The second core, F/D/custom
instructions, ROM/XIP boot, PSRAM, all other GPIO pad routes, interrupts, other
alternate functions, sleep behavior, and board peripherals remain outside the
model.
