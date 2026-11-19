# STM32F103 continuous circular ADC DMA first run

The exact imported `stm32f103` target booted 50,000,000 instructions to
`max_steps`, but its new continuous circular ADC DMA smoke failed. The DMA
half/complete flags, IRQ, two sample values, channel enable bit, and ADC EOC
clear all matched. The final CNDTR read was zero, so the smoke emitted
`ADC1 F1 CONTINUOUS CIRCULAR LAPS=2 FLAGS=1 IRQ=1 RELOAD=0 SEQ=1 ENABLED=1 EOC_CLEAR=1 FAIL`.

Command:

```sh
PATH=/usr/bin:/home/andrii/.cargo/bin:/usr/local/bin:/bin \
  CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 \
  /usr/bin/python3 scripts/run_renode_onboarding_inventory.py --only stm32f103
```

The attempt finished at 2026-11-18 21:38:01 UTC. It took 26.955 seconds,
including a 12.829-second CLI build. All metrics, logs, UART, JUnit, result,
and snapshot files from the failed attempt are preserved in this directory.
The generated aggregate temporarily recorded 225/245 passes, 1 measured
failure, and 19 not-run descriptors. The prior passing STM32F103 measurement
is preserved separately in the older attempt history.
