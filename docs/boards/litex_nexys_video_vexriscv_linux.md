# LiteX Nexys Video VexRiscv smoke evidence

The imported system is based on Renode's
[`litex_nexys_video_vexriscv_linux.repl`](https://github.com/renode/renode/blob/master/platforms/cpus/litex_nexys_video_vexriscv_linux.repl).
The measured LabWired profile includes one RV32IM execution context, the 64 KiB
ROM at `0x00000000`, the 8 KiB SRAM at `0x10000000`, LiteX UART, and LiteX timer.
The upstream CPU's atomic extension, DDR/Linux boot, CLINT, PLIC, SD card, and
framebuffer are outside this profile.

The local CLI smoke executed **4,000,000 instructions** and passed both smoke
assertions: ordered UART output including the LiteX timer one-shot status, and
the configured `max_steps` stop. JUnit records three cases (the run plus two
assertions) with zero failures. This runs the committed bare-metal fixture; it
does not boot Linux or claim Renode execution or hardware confirmation.

Evidence: [`metrics/litex_nexys_video_vexriscv_linux/`](../coverage/renode-onboarding/metrics/litex_nexys_video_vexriscv_linux/).

The first attempt used the existing 32 KiB LiteX firmware stack map against the
imported 8 KiB SRAM and stopped on a memory access violation before completing
the smoke. That attempt is retained under
[`attempt-history/`](../coverage/renode-onboarding/attempt-history/litex_nexys_video_vexriscv_linux/2026-11-06-sram-stack-mismatch/).
