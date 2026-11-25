# Renode onboarding evidence: xtensa-sample-controller

The imported board descriptor remains `verified: false`. Renode's [platform file](https://github.com/renode/renode/blob/master/platforms/cpus/xtensa-sample-controller.repl) defines a Cadence Xtensa `sample_controller` CPU (not an ESP32) at 100 MHz, a 64 MiB `rom` at `0x50000000` (the reset vector), `instram0` (128 KiB) at `0x40000000`, `dataram0` at `0x3FFE0000`, `dataram1` at `0x3FFC0000` (128 KiB each), a 128 MiB `ram` at `0x60000000`, and a SIMCALL semihosting UART.

## Measured run

- Status: **booted and smoke-passing**.
- Firmware: `tests/fixtures/xtensa-sample-controller-cpu-smoke.elf`, assembled
  from `examples/renode-onboarding/xtensa-sample-controller-cpu-smoke.S` by
  `scripts/build_xtensa_sample_controller_cpu_smoke.sh`, 100,000 instructions
  from the `0x50000000` reset vector.
- Assertions: pass word `0x5A5C003F` at `0x40000000` (counted loop, CALL0/RET,
  L32R rom literal, byte/halfword stores and loads, shifts/EXTUI, signed and
  unsigned branches), the loop sum 5050 at `0x40000004`, and the `max_steps`
  stop reason (3/3 passed).
- Evidence: [`docs/coverage/renode-onboarding/metrics/xtensa-sample-controller/`](../coverage/renode-onboarding/metrics/xtensa-sample-controller/)
  and the generated [Renode onboarding scoreboard](../coverage/renode-onboarding-scoreboard.md).

The first run is preserved at
`docs/coverage/renode-onboarding/attempt-history/xtensa-sample-controller/2026-11-08-esp32-rom-thunk-overlay/`:
`labwired test` built every Xtensa chip as an ESP32, so the ESP32 ROM-thunk
bank (`BREAK 1,14`, reads `0x41E0`) shadowed `instram0`; the CPU checks that
need no RAM passed (a4 = `0x5A5C0033`) but the RAM checks failed (1/3).
Non-Espressif Xtensa chips (`ChipDescriptor::is_espressif_xtensa` false) now
get only their descriptor's map and one core started at the ELF entry.

## Profile boundary

The LabWired profile claims the Xtensa core-ISA execution path (with the
code-density forms the assembler emits) on the LabWired LX7 interpreter, the
`rom` window and `instram0`. Not claimed: `dataram0`, `dataram1`, the 128 MiB
`ram`, the SIMCALL semihosting UART (the core has no SIMCALL), interrupts,
exceptions and windowed calls, MUL and other configuration options, and
any difference between the LX7 interpreter and the `sample_controller`
configuration Renode runs. Renode's Zephyr demos were not run here. This is
repository CLI simulation evidence, not hardware confirmation.
