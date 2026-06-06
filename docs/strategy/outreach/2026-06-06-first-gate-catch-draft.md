# Draft — "The simulation gate's first catch was our own CI" (proof-artifact note)

**Status:** DRAFT, do not publish until the gate is green on main (run 3 pending) and
the claim below is re-verified against the final logs. Per wedge v2 rule: no claim
without its artifact.

**Artifacts:** SpiceDispenser commits dc8057c → 141224b; GH Actions runs
27049765671 (fail) / 27049922536 (fix); sim-gate.log artifacts on both runs.

## The story (honest version)

We wired a simulation gate into the CI of a real shipping product (SpiceDispenser,
ESP32-S3): every PR builds the firmware, merges the exact factory image layout that
gets flashed to the bench board (verified byte-identical), and rom-boots it on the
labwired faithful chip model — real boot ROM, dual core, I2C/PCA9685 twin. The gate
demands positive evidence: faithful ROM loaded, APP_CPU released, servo command
observed on the simulated I2C bus.

First run: **FAIL — no servo command.** Not a firmware bug, and not a simulator bug:
the gate had caught that **CI was building different firmware than the bench**. The
project builds with the pioarduino espressif32 fork (Arduino core 3.x) locally;
`platform = espressif32` in CI silently resolved to the official registry platform
(core 2.x) — different bootloader, different boot flow, different binary. Every
"green build" before this gate existed would have been testing the wrong artifact.

The deterministic simulator made a toolchain divergence *visible as a behavioral
diff* within one boot. That's the point: the gate doesn't trust exit codes; it
demands the firmware demonstrably do its job on simulated silicon.

## Why this is the wedge demo (internal note)

- Catch class: reproducibility/toolchain — below even "driver bring-up" in the
  catch-class ladder, but real, and it happened on run #1 of the very first install.
- The stronger artifact (real firmware-logic regression caught) is still pending;
  this note ships only as a supporting story, not the headline.
- Channels when ready: labwired build-in-public + Interrupt-style write-up.
