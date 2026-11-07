# Renode onboarding scope: `cortex_a53_virtio`

The imported descriptor is narrowed to one Cortex-A53 processing element and
128 MiB of DDR at `0x40000000`. It no longer advertises flash, a generic timer,
PL011, or virtio entropy: this engine has not demonstrated those devices on
this profile. The platform remains `verified: false`.

The CPU-only direct-ELF smoke uses the shared AArch64 integer-loop fixture for
100,000 instructions and checks the `max_steps` stop. It performs no MMIO, so
this result does not cover reset-vector boot, Linux boot, peripherals, or the
whole board. It is local LabWired CLI evidence, not a hosted `labwired_verify`
verdict, Renode execution, or hardware confirmation.

Evidence is recorded under
[`docs/coverage/renode-onboarding/metrics/cortex_a53_virtio/`](../coverage/renode-onboarding/metrics/cortex_a53_virtio/)
and in the [generated scoreboard](../coverage/renode-onboarding-scoreboard.md).
