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

The two earlier failed integration attempts for repeated NVIC pending during
the CPU-serviced scan remain under
`docs/coverage/renode-onboarding/attempt-history/stm32f103/`. The passing
smoke claims the first-rank pending bit only. It does not exercise continuous
ADC scans, circular DMA, other DMA routes, or physical hardware behavior.

Register behavior follows ST RM0008 Rev 21 §§11.3.8, 11.12.3, 11.12.9–11.12.11,
and 13.3.4: [STM32F1 reference manual](https://www.st.com/resource/en/reference_manual/rm0008-stm32f103xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf).
