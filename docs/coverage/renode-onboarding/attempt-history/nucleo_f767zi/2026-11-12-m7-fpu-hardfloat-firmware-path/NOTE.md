# NUCLEO-F767ZI hard-float firmware path mismatch

The target-only onboarding runner rebuilt the CLI and firmware, then the smoke runner failed before loading firmware because `nucleo_f767zi-smoke.yaml` still referenced the old `thumbv7em-none-eabi` output path after its build target changed to `thumbv7em-none-eabihf` for the M7 FPU check.

No Renode firmware execution occurred. This is preserved as a runner configuration failure, not counted as a Cortex-M7 behavior result. The smoke YAML path was corrected before the target was rerun.
