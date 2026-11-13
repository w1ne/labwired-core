# Validation Report: nxp-frdm-imx8mplus

**Architecture:** AArch64, one Cortex-A53 PE in the measured profile.

## Local CLI Onboarding Smoke

**Status:** Passed, 2/2 assertions. The committed fixture
`tests/fixtures/imx8mp-a53-uart4-smoke.elf` (source
`examples/renode-onboarding/imx8mp-a53-mxcuart-smoke.S`, built by
`scripts/build_imx8mp_a53_mxcuart_smoke.sh`) runs from DDR on one Cortex-A53
PE. It checks that UART4 USR1 reads TRDY set and RRDY clear after reset; a
window answering zeros prints `USR1=BAD` instead. It then sets UCR2/UCR1,
writes `IMX8MP A53 MXCUART SMOKE USR1=OK` through UTXD while polling
USR1.TRDY, and spins until the 100,000-instruction `max_steps` stop.

The chip profile declares only UART4 (`MxcUart` at `0x30A60000`), which
Renode's `nxp-imx8mplus_zephyr.resc` uses as the console. It runs on the
generic UART engine with the i.MX layout (UTXD `0x40`, URXD `0x00`, USR1
`0x94`). USR2, UTS, the FIFOs, the baud generator and the interrupt line are
not claimed. The other three Cortex-A53 cores, the Cortex-M7, the other UARTs
and peripherals, and the board's boot flow are not tested. The onboarding
descriptor is `verified: false` because the whole platform is not covered.

Negative control: the same ELF on the previous CPU-only chip profile
(`peripherals: []`) fails 0/2 with a memory violation at `0x30a60094`
(USR1).

Evidence: [onboarding metrics](../coverage/renode-onboarding/metrics/nxp-frdm-imx8mplus/onboarding-metrics.json), [simulation result](../coverage/renode-onboarding/metrics/nxp-frdm-imx8mplus/simulation-output/result.json), [JUnit assertions](../coverage/renode-onboarding/metrics/nxp-frdm-imx8mplus/simulation-output/junit.xml), [run log](../coverage/renode-onboarding/metrics/nxp-frdm-imx8mplus/logs/run_smoke.log).
