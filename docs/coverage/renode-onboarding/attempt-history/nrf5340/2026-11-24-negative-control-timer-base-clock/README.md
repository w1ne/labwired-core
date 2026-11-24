# nRF53/nRF54L negative control: TIMER counts CPU cycles (2026-11-24)

The nRF5340 application core runs at 128 MHz. Its TIMER0..2 count PCLK16M
(16 MHz; nRF5340 PS v1.5). The nRF54L parts run at 128 MHz: TIMER10 counts
HFXO at 32 MHz, TIMER2x counts PCLK16M at 16 MHz, and nRF54LM20A TIMER00
counts the 128 MHz HF PLL. Sources: Nordic MDK `TIMERxx_PCLK_MHZ`
(`nrf54l15_application_peripherals.h`, `nrf54lm20a_application_peripherals.h`,
`nrf54l05/10_application_peripherals.h`) and the Zephyr devicetree `clocks`
property (`nrf54l_05_10_15.dtsi`).

Before this change `Nrf52Timer` advanced one base tick per CPU cycle when the
descriptor had no `config: { peripheral_hz }`. On these parts that made TIMER
run 4x or 8x fast, silently.

The five `*.yaml` files here are the descriptors from before the change
(`origin/main` f1aa89a5c2). The same test binary (`cargo-test.log`) fails both
checks:

```
nrf5340.yaml: timer `timer0` has no `peripheral_hz`, which is only correct
when its base clock (16000000 Hz) equals the core clock (128000000 Hz)
nrf5340.yaml `timer0`: after 640 CPU cycles the counter is 640, expected 80
test result: FAILED. 0 passed; 2 failed
```

With the descriptors in `configs/chips/`, the same binary passes 3/3
(`after.log`, 2026-11-24):

```
test nrf_timer_base_clock::every_nrf_timer_declares_a_base_clock_when_it_differs_from_the_core ... ok
test nrf_timer_base_clock::nrf5340_timers_expose_six_compare_channels ... ok
test nrf_timer_base_clock::nrf_timers_count_their_base_clock_not_the_core_clock ... ok
```
