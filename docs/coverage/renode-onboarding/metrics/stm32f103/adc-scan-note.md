# STM32F103 ADC scan scope

The exact-import smoke checks a two-rank PA0/PA1 regular scan with EOC on each
rank, first-rank ADC IRQ pending, and EOC clear after the CPU reads DR. A
13-rank one-shot scan then enables ADC1 DMA and drains results through DMA1
channel 1 into SRAM. The alternating IN0/IN1 values cover ranks 1–6 in SQR3,
ranks 7–12 in SQR2, and rank 13 in SQR1; the smoke checks all 13 values, DMA
transfer complete, CNDTR reaching zero, and DMA clearing EOC by reading DR.

The bounded model emits ADC1's DMA request on each completed regular rank only
when CR2.DMA is set. The imported chip profile routes `adc1_rx` to DMA1 channel
1, the STM32F1 fixed mapping. The unit regression checks the per-rank request;
the imported-system smoke checks the request-to-memory path.

The smoke also runs software-started two-rank scans with DMA1 channel 1 in
circular mode. With TCIE disabled and HTIE enabled, two laps must set HTIF and
pend DMA1 channel 1's IRQ, then complete with CNDTR reloaded to 2. A third lap
enables both interrupt sources and checks the combined GIF/HTIF/TCIF state,
independent HTIF and TCIF clears, CGIF clearing all channel flags, and NVIC
pending clear behavior. It checks every result buffer, the channel remaining
enabled, and EOC clearing after DMA reads DR. Larger-buffer half-transfer
timing and other DMA routes remain unclaimed.

A separate continuous-mode run starts one two-rank scan with CONT set and
drains two groups through a four-item normal-mode DMA transfer. The smoke
checks both PA0/PA1 sequences, CNDTR zero, and that later ADC requests do not
overwrite the result buffer after normal-mode transfer complete. DMA1 channel
1 remains enabled at TC; firmware must disable it before reloading CNDTR.
See RM0008 Rev 21 §13.3.1: [STM32F1 reference manual](https://www.st.com/resource/en/reference_manual/rm0008-stm32f103xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf).

A second continuous-mode run combines the two-rank scan with circular DMA.
It checks two laps into the same two-item PA0/PA1 buffer, both HTIF and TCIF,
DMA1 channel 1's pending IRQ, CNDTR reload, the enabled channel, and EOC
clearing after DMA reads DR. This covers the combined continuous-request and
circular-reload path; it does not measure sample timing or long-duration ADC
throughput. Reload is sampled at each transfer-complete boundary while CONT
is still running; a later CNDTR read can reflect the next in-flight group.
The initial final-count assertion failure is preserved under
`docs/coverage/renode-onboarding/attempt-history/stm32f103/2026-11-18-continuous-circular-reload-missing/`.

The two earlier failed integration attempts for repeated NVIC pending during
the CPU-serviced scan remain under
`docs/coverage/renode-onboarding/attempt-history/stm32f103/`. The passing
smoke claims the first-rank ADC pending bit only. It does not exercise
long-duration continuous ADC operation, larger-buffer half-transfer timing,
other DMA routes, or physical hardware behavior. Register behavior follows ST RM0008 Rev 21
§§11.3.8, 11.12.3, 11.12.9–11.12.11, 13.3.1, 13.3.4, and 13.4.1–13.4.2:
[STM32F1 reference manual](https://www.st.com/resource/en/reference_manual/rm0008-stm32f103xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf).
