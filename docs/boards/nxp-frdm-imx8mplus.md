# Validation Report: nxp-frdm-imx8mplus

**Architecture:** AArch64, one Cortex-A53 PE in the measured profile.

## Local CLI Onboarding Smoke

**Status:** Passed. The committed AArch64 fixture executed 100,000 instructions
from DDR and stopped at the configured `max_steps` limit. It made no MMIO
accesses.

This profile tests CPU instruction execution and the mapped DDR window only. It
does not test the other three Cortex-A53 cores, the Cortex-M7, peripherals, or
the board's boot flow. No peripheral is currently declared in the imported chip
profile.

Evidence: [onboarding metrics](../coverage/renode-onboarding/metrics/nxp-frdm-imx8mplus/onboarding-metrics.json), [simulation result](../coverage/renode-onboarding/metrics/nxp-frdm-imx8mplus/simulation-output/result.json), [JUnit assertions](../coverage/renode-onboarding/metrics/nxp-frdm-imx8mplus/simulation-output/junit.xml), [run log](../coverage/renode-onboarding/metrics/nxp-frdm-imx8mplus/logs/run_smoke.log).
