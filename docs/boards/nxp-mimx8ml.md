# NXP i.MX 8M Plus imported profile

## Measured scope

This imported fragment is narrowed to one Cortex-A53 processing element, the
imported 3 GiB DDR window at `0x40000000`, and UART2 (`MxcUart` at
`0x30890000`), which Renode's `nxp-imx8mplus_linux.resc` uses as the console.
UART2 runs on the generic UART engine with the i.MX layout: UTXD `0x40`, URXD
`0x00`, USR1 `0x94` (TRDY/RRDY). USR2, UTS, the FIFOs, the baud generator and
the interrupt line are not claimed. The i.MX 8M Plus also includes other A53
cores and a Cortex-M7. The other cores, the other UARTs and peripherals, and
the boot flow are outside this profile. NXP identifies the MIMX8ML family as four Cortex-A53
cores and one Cortex-M7: [MIMX8ML8 product information](https://www.nxp.com/part/MIMX8ML8CVNKZAB).

## Smoke evidence

The committed fixture `tests/fixtures/imx8mp-a53-uart2-smoke.elf` (source
`examples/renode-onboarding/imx8mp-a53-mxcuart-smoke.S`, built by
`scripts/build_imx8mp_a53_mxcuart_smoke.sh`) runs from DDR. It first checks
that USR1 reads TRDY set and RRDY clear after reset; a window answering zeros
prints `USR1=BAD` instead of the expected banner. It then sets UCR2/UCR1,
writes `IMX8MP A53 MXCUART SMOKE USR1=OK` through UTXD while polling
USR1.TRDY, and spins until the 100,000-instruction `max_steps` stop. Both
assertions (UART banner and stop reason) pass. The negative control was run
on the sibling `nxp-frdm-imx8mplus` target (same source, UART4): that ELF on
the previous CPU-only chip profile fails 0/2 with a memory violation at
USR1.

This is local LabWired CLI evidence for the bounded AArch64 execution,
memory and UART2 polled-TX profile. It is not full i.MX 8M Plus CPU-cluster
validation, a hosted `labwired_verify` verdict, Renode execution, validation
of other peripherals, reset-vector boot, or hardware confirmation. The
whole-platform onboarding descriptor remains `verified: false`.

Evidence: [onboarding metrics](../coverage/renode-onboarding/metrics/nxp-mimx8ml/onboarding-metrics.json), [JUnit assertions](../coverage/renode-onboarding/metrics/nxp-mimx8ml/simulation-output/junit.xml), [simulation result](../coverage/renode-onboarding/metrics/nxp-mimx8ml/simulation-output/result.json), [run log](../coverage/renode-onboarding/metrics/nxp-mimx8ml/logs/run_smoke.log).
