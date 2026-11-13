# Passing SysTick PRIMASK measurement before clear-path coverage

The exact imported `stm32f072b_discovery` system booted 4,000,000 instructions
and passed all 5 smoke assertions. UART confirmed that ICSR reported SysTick
pending while PRIMASK blocked handler entry, then the handler ran after
interrupts were unmasked. This passing measurement is retained before adding a
separate PENDSTCLR assertion to the same target.
