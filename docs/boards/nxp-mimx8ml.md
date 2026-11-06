# NXP i.MX 8M Plus imported profile

## Measured scope

This imported fragment is narrowed to one Cortex-A53 processing element and
the imported 3 GiB DDR window at `0x40000000`. The i.MX 8M Plus also includes
other A53 cores and a Cortex-M7; the other cores, peripherals, and boot flow
are outside this profile. NXP identifies the MIMX8ML family as four Cortex-A53
cores and one Cortex-M7: [MIMX8ML8 product information](https://www.nxp.com/part/MIMX8ML8CVNKZAB).

## Smoke evidence

The committed AArch64 integer-loop fixture runs directly from DDR for 100,000
instructions and asserts the configured `max_steps` stop. It performs no
MMIO. This is local LabWired CLI evidence for the bounded AArch64 execution
and memory profile. It is not full i.MX 8M Plus CPU-cluster validation, a
hosted `labwired_verify` verdict, Renode execution, peripheral validation,
reset-vector boot, or hardware confirmation. The whole-platform onboarding
descriptor remains `verified: false`.

Evidence: [onboarding metrics](../coverage/renode-onboarding/metrics/nxp-mimx8ml/onboarding-metrics.json), [JUnit assertions](../coverage/renode-onboarding/metrics/nxp-mimx8ml/simulation-output/junit.xml), [simulation result](../coverage/renode-onboarding/metrics/nxp-mimx8ml/simulation-output/result.json), [run log](../coverage/renode-onboarding/metrics/nxp-mimx8ml/logs/run_smoke.log).
