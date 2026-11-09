# SAM4S8B USART0 TXRDY IRQ attempt with stale summary mask

The one-target onboarding run booted 1,500,000 instructions. The USART0
TXRDY assertion passed (`imr=2`, IRQ 14 pended, then released), and the
summary memory assertion observed `0x5A401FFF`. The firmware still printed
`SAM4S8B FAIL` because the shared family runner compared against the original
12-bit mask `0xFFF`; that final UART assertion failed.

This failed run is retained before correcting the expected summary mask. The
final measurement is in `docs/coverage/renode-onboarding/metrics/sam4s8b/`.
