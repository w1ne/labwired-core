# Attempt 2: second IRQ sampled before DR read

The imported `stm32f103` system again booted all 50,000,000 instructions. Both
regular ranks produced the expected ADC codes and EOC/DR-clear checks passed.
The first ADC IRQ was pending, but the second was not visible in NVIC ISPR even
when sampled before reading the second DR value. The broader smoke assertion
failed and this evidence is retained. The passing scan smoke is scoped to rank
ordering, EOC, DR-clear behavior, and the first interrupt; repeated NVIC pending
is not claimed by this attempt.
