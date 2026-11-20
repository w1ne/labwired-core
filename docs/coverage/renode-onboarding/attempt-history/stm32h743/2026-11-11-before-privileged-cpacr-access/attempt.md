# Prior passing STM32H743 measurement

Archived before the privileged-only CPACR smoke extension. This is the prior committed passing target measurement from `HEAD` (Round 537), retained as the exact source snapshot for comparison with the updated run.

- Imported system: `configs/systems/onboarding/stm32h743.yaml`
- Target runner: `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 /usr/bin/python3 scripts/run_renode_onboarding_inventory.py --only stm32h743`
- Prior assertions: 18 smoke assertions; 19 JUnit cases including run; zero failures/errors.
- Prior UART included the NOCP fault in unprivileged mode and successful full-access unprivileged FP round-trip (`result=0000005A`).
