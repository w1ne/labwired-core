# STM32F103 ADC scan scope

The exact-import smoke checks a two-rank PA0/PA1 regular scan with EOC on each
rank, first-rank ADC IRQ pending, and EOC clear after the CPU reads DR. A
13-rank one-shot scan then enables ADC1 DMA and drains results through DMA1
channel 1 into SRAM. The alternating IN0/IN1 values cover ranks 1–6 in SQR3,
ranks 7–12 in SQR2, and rank 13 in SQR1; the smoke checks all 13 values, DMA
transfer complete, CNDTR reaching zero, and DMA clearing EOC by reading DR.
With an odd initial CNDTR of 13, it also observes HTIF and DMA1 channel 1's
pending IRQ while transfers remain, clears the half-transfer flag, then checks
that completion occurs without TCIE pending another IRQ. The DMA model now
tracks that half-transfer event once per transfer, so clearing HTIF cannot
retrigger it on later items; circular reload arms it for the next lap. The
smoke also disables the completed channel, loads a new odd transfer count,
and verifies that HTIF and its IRQ fire again for the new transfer. It clears
that half-transfer flag, then checks the fresh transfer reaches CNDTR zero with
TCIF and the TCIE-driven IRQ pending. A focused DMA1 unit test also checks that
clearing TCIF before re-enabling the channel allows a second one-item transfer
to raise TCIF and its IRQ. The UART records the remaining count observed when
HTIF is first seen; this check does not claim cycle-accurate interrupt latency.

The bounded model emits ADC1's DMA request on each completed regular rank only
when CR2.DMA is set. The imported chip profile routes `adc1_rx` to DMA1 channel
1, the STM32F1 fixed mapping. The unit regression checks the per-rank request;
the imported-system smoke checks the request-to-memory path.

The imported chip profile routes DMA1 channels 1–7 to their STM32F1 IRQ lines
11–17 and routes `usart2_rx` to channel 6 and `usart2_tx` to channel 7. The
smoke leaves two injected USART2 bytes queued after its CPU-read RX gate,
enables DMA1 channel 6 with DMAR clear, and checks that CNDTR, RXNE, TCIF6,
and IRQ16 do not change. It
then enables CR3.DMAR and checks the two bytes in SRAM, CNDTR zero, TCIF6,
IRQ16 pending, RXNE clear, and that clearing the DMA flags deasserts the
channel-6 line. It also sends the three-byte `DMA` payload from SRAM through
USART2's `CR3.DMAT` request on channel 7. After clearing TCIF7 and its IRQ, it
clears DMAT, disables channel 7, loads a different two-byte `R2` source and
count, then re-enables the channel and sets DMAT for the second transfer. This
follows the STM32F1 HAL normal-mode completion path, which clears DMAT when the
DMA count completes. The smoke checks the USART2 TX stream contains `DMAR2`,
both CNDTR values reach zero, both TCIF7/IRQ17 completions occur, and clearing
each completion lowers the channel line. This verifies two sequential
transfers and software re-arm; it does not claim baud-timing coverage. See
[ST's STM32F1 HAL UART driver](https://github.com/STMicroelectronics/stm32f1xx-hal-driver/blob/master/Src/stm32f1xx_hal_uart.c#L2805-L2818).

The smoke also runs software-started two-rank scans with DMA1 channel 1 in
circular mode. With TCIE disabled and HTIE enabled, two laps must set HTIF and
pend DMA1 channel 1's IRQ, then complete with CNDTR reloaded to 2. A third lap
enables both interrupt sources and checks the combined GIF/HTIF/TCIF state,
independent HTIF and TCIF clears, CGIF clearing all channel flags, and NVIC
pending clear behavior. It checks every result buffer, the channel remaining
enabled, and EOC clearing after DMA reads DR. A four-item circular buffer adds
an explicit halfway boundary check: HTIF and its IRQ are observed at CNDTR=2
with TCIF absent. After firmware clears HTIF, the second two-rank scan completes
the same transfer with TCIF set and HTIF still clear; the following circular
lap raises HTIF again at CNDTR=2. Other DMA routes remain unclaimed.

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
long-duration continuous ADC operation, larger DMA buffer sizes, other DMA
routes, or physical hardware behavior. Register behavior follows ST RM0008 Rev 21
§§11.3.8, 11.12.3, 11.12.9–11.12.11, 13.3.1, 13.3.4, and 13.4.1–13.4.2:
[STM32F1 reference manual](https://www.st.com/resource/en/reference_manual/rm0008-stm32f103xx-advanced-armbased-32bit-mcus-stmicroelectronics.pdf).
