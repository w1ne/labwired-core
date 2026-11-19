# lpc2294 — negative control: this round's descriptor without port0..port3 (2026-11-09)

Run: this round's firmware against this round's descriptor with the four
`lpc2xxx` GPIO ports removed (`chip.yaml`).

Result: FAIL 0/3. UART0 prints "LPC2294 checks PPPP" (SCB, PINSEL0, UART0
DLAB, UART0 LSR pass), then check 5 stops on "Memory access violation at
0xe0028018" (IO1DIR). The GPIO checks depend on the ports being declared.
