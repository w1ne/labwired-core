# Failed STM32F401 nested active-FPU build attempt

The target-only onboarding run failed during `build_firmware` because LLVM rejected VFP instructions inside the new naked assembly handlers. The metric records `run_attempted: false`; the firmware did not boot and no smoke was executed. The existing `run_smoke.log` and simulation output were not copied because they belong to the prior passing run, preserved under `../2026-11-12-before-nested-active-fpu-smoke/`.
