# riscv_verilated_uartlite negative control: uart removed (2026-11-08)

Same firmware and smoke script, with `uart` removed from the chip (the
`riscv_verilated_uartlite.yaml` copy here, loaded through a one-line system
manifest `chip: <abs path>/riscv_verilated_uartlite.yaml` with
`labwired test --script`). FAIL 1/6: the first UART access (the CTRL write at
0x70000008) is a memory access violation and the run stops (the CLI reports
0 steps), so no UART bytes appear, the CTRL-read word at 0x60001004 keeps its
0xDEADBEEF sentinel, the STAT and pass words are never written, and the run
does not reach max_steps. Only the MUL word at 0x60001000 (written before any
peripheral access) holds. The UART checks therefore depend on the modelled
uart.
