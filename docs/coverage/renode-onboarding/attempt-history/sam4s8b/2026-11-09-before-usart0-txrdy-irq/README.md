# SAM4S8B measurement before USART0 TXRDY IRQ smoke

This preserves the prior exact-import smoke evidence before extending the
SAM4S8B profile and firmware to exercise USART0 TXRDY IRQ 14. The prior smoke
booted 1,500,000 instructions and passed its three onboarding assertions; it
did not claim or test the USART0 interrupt route.

The current measurement remains under
`docs/coverage/renode-onboarding/metrics/sam4s8b/`.
