# STM32F103 ADC scan scope

The smoke verifies a two-rank PA0/PA1 regular conversion sequence, each EOC,
the first ADC IRQ pending bit, and EOC clearing after the first DR read. The
peripheral unit test observes the EOC interrupt result for both ranks. The
imported-system runs did not show NVIC ISPR pending for the second rank, even
when sampled before reading its DR, so repeated NVIC pending is not claimed by
this smoke. The two failed integration attempts are retained under
`docs/coverage/renode-onboarding/attempt-history/stm32f103/`.

This evidence covers scan ordering and per-rank EOC in the bounded model. It
tests a two-rank one-shot group using SQR3; ranks in SQR2/SQR1 and continuous
scan are implemented but not exercised by this round. It does not model F1 ADC
DMA transfer behavior, which RM0008 requires to preserve multiple regular
results without software servicing ADC_DR between ranks.
