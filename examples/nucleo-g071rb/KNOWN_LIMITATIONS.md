# Known limitations — NUCLEO-G071RB (STM32G071RB)

**SIM-DERIVED.** No physical NUCLEO-G071RB has been connected for this port.
Every register number traces to RM0444 / DS12232 / ST's CMSIS `stm32g071xx.h`;
the checks below are simulator-side. This file is the L2 requirement of
[`docs/target_support_rubric.md`](../../docs/target_support_rubric.md) and is
the honest boundary around the L3 claim. Reproduce everything in
[`VALIDATION.md`](VALIDATION.md); the board page is
[`docs/boards/nucleo-g071rb.md`](../../docs/boards/nucleo-g071rb.md).

## Not modelled

- **Silicon bench diff.** There is no executing-fidelity differential (no
  walk-vs-scheduler oracle, no silicon capture) and no `walk_deleted` claim.
  `dbgmcu.idcode` (`0x460`) is ST's **published** G07x/G08x DEV_ID constant,
  not a value read off a die.
- **PLL frequency.** `PLLON` → `PLLRDY` is modelled; the programmed output
  frequency is not, so clock-tree maths beyond the HSI16 path is unverified.
- **DAC1 analog output.** Register window only; no analog voltage is produced.
- **External analog input.** ADC1 converts the model's fixed internal source
  by value (3723 @ 12-bit, 930 @ 10-bit, scaling with `CFGR.RES`) — no pin,
  no sample-and-hold timing.
- **External I2C/SPI devices.** Controller-level only: no external slave is
  wired; the I2C check deliberately exercises an absent-slave NACK.
- **UCPD, CEC, VREFBUF, COMP, DMAMUX.** Not declared / not modelled.
- **ARMv6-M enforcement.** The engine decodes the full Thumb-2 set; the
  `thumbv6m-none-eabi` toolchain is the only ISA guardrail.
- **Cycle-accurate timing.** Baud/timer timing is functional, not
  cycle-accurate; nothing here has been scoped against a logic analyser.

## Partially modelled

- **Only the first instance per class is swept.** Tier-1 exercises GPIOA
  (GPIOC only as the clock-gate witness), USART2 (the console), I2C1, SPI1,
  ADC1, DMA1 channel 1, TIM1 and TIM2, IWDG and RTC. The declared second
  instances — GPIOB/C/D/F, USART1/3/4, LPUART1, I2C2, SPI2, LPTIM2 — are
  register-declared but carry no behavioural proof.
- **PWM depth.** TIM1 advanced is proven at the flag level: `UG` latches
  CC2..4IF/CC5..6IF, and a running counter raises CC1IF after `CCR1` (with
  `BDTR.MOE` set). There is no waveform sweep: complementary outputs, dead
  time, break input, OCxM modes beyond PWM1, and channel polarity are not
  exercised.
- **LPTIM1/2, WWDG, CRC, EXTI/SYSCFG.** Declared with real bases/IRQs and
  family models; not G0-diffed and not touched by the Tier-1 fixture. EXTI is
  a single-bank `stm32f1`-profile model.
- **Timers.** TIM3/6/7/14/15/16/17 are declared with G0 widths (RM0444
  Table 129) but only TIM2 (32-bit ARR/UIF/CEN) and TIM1 (advanced flags) are
  exercised.
- **Interrupt delivery.** Tier-1 proves one NVIC **software-pended** vector
  (IRQ 30). No claim is made that any peripheral's own IRQ line reaches the
  NVIC on this part.
- **DMA.** One mem-to-mem byte copy on DMA1 channel 1 (MINC+PINC, TCIF1);
  DMAMUX and peripheral-request transfers are untested.
- **Watchdog.** IWDG PR/RLR write-protection and unlock/latch are proven;
  timeout/reset behaviour and WWDG are not.
- **RTC.** `DR` reset, `WPR` unlock and `TR` round-trip only; the calendar,
  clock source and alarm paths are untested.
- **Register coverage is a lower bound.** The 52.5% modelled figure
  (`docs/coverage/register-modeling.json`) under-counts write-only and
  read-only-reset-0 registers by construction.

## Proven at L3

For the documented scenarios in `VALIDATION.md` (Tier-1 fixture
`tests/fixtures/tier1/stm32g071.elf`, committed blob):

- The six tier-1 classes all **pass**: clock/RCC, GPIO, UART (implicit
  console), Timer, DMA, Interrupt delivery.
- The extended row also passes: `pwm`, `i2c`, `spi`, `adc`, `wdt`, `rtc` —
  eleven explicit classes plus implicit `uart` (all twelve matrix cells),
  `TIER1 done` observed.
- Each gated class is poked **while its RCC gate is off** and must read dead
  before the enable, so a wrong G0 gate offset fails the class.
- The L1 smoke (`firmware_survival::test_nucleo_g071rb_smoke_survival`) and
  config-build gate (`board_config_gates::stm32g071_from_config_builds`) still
  pass; register-vs-SVD conformance is green.

Beyond those scenarios, assume nothing. Anything not listed above is either
partially modelled or not modelled, and must be treated as the bench's job.
