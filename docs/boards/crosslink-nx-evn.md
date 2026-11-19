# CrossLink-NX EVN onboarding scope

The imported platform is a LiteX system with a VexRiscv core. Renode's board
description gives it a `rv32im_zicsr_zifencei` CPU, a 32 KiB ROM at `0x0`, and
8 MiB SRAM at `0x40000000` ([upstream board REPL](https://github.com/renode/renode/blob/master/platforms/boards/crosslink-nx-evn.repl)).

LabWired's measured profile claims only its generic RV32IM integer subset and
those memory windows. The exact imported-system direct-ELF smoke executes
100,000 instructions, stores `17 * 19` at `0x40001000`, and checks the SRAM
readback. It does not validate the reset flow, Zicsr or Zifencei instructions,
UART, timer, LED controller, or any other board peripheral. The complete board
remains unverified.

Run evidence: `docs/coverage/renode-onboarding/metrics/crosslink-nx-evn/`.
