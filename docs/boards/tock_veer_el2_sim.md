# Renode onboarding evidence: tock_veer_el2_sim

The imported board descriptor remains `verified: false`. Renode's [platform file](https://github.com/renode/renode/blob/master/platforms/cpus/tock_veer_el2_sim.repl) defines a VeeR EL2 CPU, a 5 MiB ROM at `0x0`, a 3 MiB ROM at `0x20000000`, a 3 MiB flash window at `0x20300000`, 3 MiB of RAM at `0x20600000`, a CLINT at `0x02000000`, the VeeR EL2 PIC at `0xF00C0000` and a 4 KiB `htif_mem` window at `0xD0580000`. Renode's [`veer_el2-tock.resc`](https://github.com/renode/renode/blob/master/scripts/single-node/veer_el2-tock.resc) turns byte and word writes to `0xD0580000` into console characters; Tock's VeeR EL2 board uses that address as its console.

## Measured run

- Status: **booted and smoke-passing**.
- Firmware: `firmware-tock-veer-el2-smoke`, RV32IMC, 100,000 instructions.
- Stop reason: `max_steps` after all checks completed.
- Assertions: the RV32IMC MUL result (0x143) in RAM, the firmware pass word
  0x7E3EE120, the banner `VeeR EL2 htif ok` on the `htif` console (written as
  byte stores, as Tock does), and the stop reason (4/4 passed).
- Evidence: [`docs/coverage/renode-onboarding/metrics/tock_veer_el2_sim/`](../coverage/renode-onboarding/metrics/tock_veer_el2_sim/)
  (build/run logs, JUnit, result, UART log and snapshot) and the generated
  [Renode onboarding scoreboard](../coverage/renode-onboarding-scoreboard.md).

The earlier CPU/RAM-only measurement is preserved at
`docs/coverage/renode-onboarding/attempt-history/tock_veer_el2_sim/2026-11-08-before-htif-smoke/`.
The failed review run before the HTIF window was mapped is preserved at
`docs/coverage/renode-onboarding/attempt-history/tock_veer_el2_sim/2026-11-08-unmapped-htif-write/`.

## Profile boundary

The LabWired onboarding profile selects the generic `rv32imc` CPU path, the
`0x20000000` ROM and the RAM window, and `htif` as a 4-byte `uart` with a
custom map: byte writes at offset 0 transmit the byte. The firmware smoke
exercises this byte-write path.

Differences from Renode that are not claimed: reads of the HTIF word return 0
here (Renode's MappedMemory reads back the last write); the rest of the 4 KiB
window is unmapped; the 0x01/0xFF "finish, pause the machine" values are not
modelled. Also not claimed: ROM0 at 0, the `0x20300000` flash window (where
the Renode script loads the Tock app), the CLINT, the VeeR EL2 PIC, the VeeR
machine-timer CSRs and any other VeeR custom CSRs. The Tock kernel and Rot13
demo that Renode's robot test runs were not run here. Simulation assumes an
ideal VDD rail; power-on and brown-out behavior were not exercised. This is
repository CLI simulation evidence, not hardware confirmation.
