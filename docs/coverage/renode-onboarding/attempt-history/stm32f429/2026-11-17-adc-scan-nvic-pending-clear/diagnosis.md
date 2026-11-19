# STM32F429 ADC scan attempt 1

The two scan results were correct (`IN1=0x03FF`, `IN2=0x07FF`), but the
firmware cleared NVIC pending after rank 1 and then expected a second pending
bit after rank 2. The F4 ADC conversion interval in this model is 15 cycles, so
the two peripheral events can coalesce into the same NVIC pending bit before
the firmware clears it. The check therefore did not isolate two separate IRQ
entries.

The smoke now checks that EOC is visible for each rank, that ADC1 IRQ 18 is
pending at the first EOC and remains pending through rank 2, and that software
can release pending after reading DR and clearing NVIC. The peripheral unit
tests check that each rank independently returns an EOC interrupt request.
