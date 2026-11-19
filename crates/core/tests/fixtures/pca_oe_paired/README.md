Generic cross-repository evidence fixture, not a saved user project.

`generated-system.yaml` was produced by LabWired app commit `f699c35cb265d4778944f3446a0d67a23960d5f3` from an explicitly authored ESP32-C3 circuit: two PCA9685 controllers at 0x40 and 0x41, twenty servo parts, shared I2C GPIO8/9, active-low OE GPIO3/4. SHA-256: `f6a359188fb7135ada94ed79072b2472a2634826a2e6bfc20fe630f1c56f854c`.

The evidence test programs the actual modeled PCA peripherals through test-only controller access; it does not run firmware, hosted WASM, the saved project, or physical hardware. The separate evidence-only test uses a concrete downcast and is not intended for the production PR.
