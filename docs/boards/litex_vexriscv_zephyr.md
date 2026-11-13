# Validation Report: litex_vexriscv_zephyr

## Measured imported-system smoke

**Status:** smoke passed on the exact imported system descriptor.

- Firmware: `firmware-litex-vexriscv-smoke`, built for `riscv32im-unknown-none-elf`.
- Execution: 4,000,000 instructions; stopped at the configured `max_steps` limit.
- Smoke checks: UART banner and ordered UART output showing the LiteX timer's
  running and zero states, followed by UART status readbacks.
- Runtime profile: RISC-V RV32IM, mapped memories, LiteX UART, and LiteX timer.

This measures the firmware path and the assertions above. It does not measure
booting Zephyr, the LiteX BIOS or SPI-flash boot path, or any omitted peripheral
such as Ethernet or MMCM. The narrowed profile contains no zero-answering
peripheral stubs.

Evidence is committed under
[`docs/coverage/renode-onboarding/metrics/litex_vexriscv_zephyr/`](../coverage/renode-onboarding/metrics/litex_vexriscv_zephyr/)
and aggregated in the
[Renode onboarding scoreboard](../coverage/renode-onboarding-scoreboard.md).
