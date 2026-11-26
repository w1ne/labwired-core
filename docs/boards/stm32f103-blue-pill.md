# STM32F103C8T6 Blue Pill

**Demand position:** STM32F103 ranks #3 at 17% of reported MCU use. The Blue
Pill is a board-level profile over the existing STM32F103C8 model.

The profile includes its reference active-low user LED on PC13, the USART1
header path on PA9/PA10, and hardware PWM on TIM3_CH1 (PA6) and TIM2_CH2 (PA1).
The GPIO smoke prints `BLUEPILL_F103C8_OK` and checks PC13 output edges; the
PWM smoke runs an unmodified STM32duino sketch and measures 1000.0 Hz at
25.10 % on PA6 and 75.29 % on PA1, with an L298N motor plant reaching
325.90 rpm. It does not claim USB device behavior, a USB bootloader, an
onboard debugger, TIM1 complementary outputs, or physical board validation.
Blue Pill clones can differ.

- [Pin map, Arduino example, and smoke command](../../examples/stm32f103-blue-pill/README.md)
- [Live ST lifecycle and retailer availability receipt](../../examples/stm32f103-blue-pill/vendor-status.md)
- [Committed local simulator evidence](../../examples/stm32f103-blue-pill/evidence/)
- [PWM waveform test and its negative controls](../../crates/core/tests/stm32/stm32f103_arduino_pwm.rs)
- [PWM receipt: what was measured, what is still stubbed](../receipts/2026-11-25-stm32f103-bluepill-pwm/README.md)

The STM32F103 chip descriptor also declares unexercised peripherals. Its USB
device and BKP windows are stubs, and WWDG is inert; this board profile does
not claim those paths. The exercised GPIOC, USART1, TIM3 and TIM2 models are
separate from those gaps. The TIM1 complementary outputs behind `PA7` are not
modelled, and the AFIO remaps are not decoded: only the default fixed timer
map is wired.

Two clock profiles exist for this board for one honest reason: `cpu_hz` is
static per run. `configs/systems/stm32f103-blue-pill.yaml` is the 8 MHz HSI
reset clock the bare-metal GPIO smoke boots at;
`examples/stm32f103-blue-pill/pwm-system.yaml` is the 72 MHz PLL clock stock
STM32duino selects. The Tier-1 grid's `pwm` cell for this chip still renders
"not modelled" because the cell follows peripheral-id class markers and this
descriptor names its timers `tim1..tim4`; that is a reporting gap, recorded in
the PWM receipt, not a capability claim.
