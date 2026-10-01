# F103 images

One firmware (`firmware/main.c`). A case passes when its marker reaches the UART. LabWired must print it. `clockbug`, `gpiobug`, and `rambug` print only when the USART clock gate, the GPIOA clock gate, and the 20 KB SRAM hold.

| case | marker |
| --- | --- |
| `control` | `BENCH_UART_OK` |
| `clockbug` | `BENCH_UART_OK` when `TXE` stays clear while USART1 is gated |
| `gpiobug` | `BENCH_GPIO_OK` when a gated GPIOA drops the ODR write |
| `rambug` | `BENCH_RAM_OK` from the fault handler on a store past 20 KB |
| `irqtime` | `BENCH_UIF_OK` |
| `nvicclear` | `BENCH_NVIC_OK` |
| `usartmux` | `BENCH_UART_OK`, and `BENCH_POISON` must not appear |

`irqtime` arms TIM2 (`ARR` 1000), spins a few dozen cycles, and prints when `SR.UIF` is still clear. `nvicclear` sets and clears the NVIC pending bit for IRQ0 while PRIMASK is set, and prints when that ISR does not run. `usartmux` prints a banner on the muxed pad, drives PA9 as GPIO push-pull, writes `BENCH_POISON`, restores the USART mux, and prints `BENCH_UART_OK`. The poison byte must not reach the pad.

The nRF52840 images live in `examples/nrf52840-fidelity-bench`. LabWired has to print every marker.

## Run

```bash
./run-benchmark.sh
```

Exits non-zero unless LabWired prints every marker, and writes
`benchmark-results.json`.

Needs `arm-none-eabi-gcc` and a built `labwired` (`cargo build -p labwired-cli`).
