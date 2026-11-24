# nRF RTC counted the 64 MHz core clock on every part (2026-11-24)

`Nrf52Rtc` converts CPU cycles into LFCLK (32.768 kHz) edges. The conversion
ratio was the compile-time 64 MHz default (`inc 8 / period 15625`) on every
chip, so the counter was only correct where the core runs at 64 MHz.

| descriptor | core | RTC counter before | after |
|---|---:|---|---|
| `nrf51822` | 16 MHz | 4x slow | correct |
| `nrf52832`, `nrf52833`, `nrf52840`, `onboarding/nrf52840` | 64 MHz | correct | byte-identical |
| `nrf5340` | 128 MHz | 2x fast | correct |

Measured with the forced peripheral walk, PRESCALER 0, `cpu_hz / 1024` CPU
cycles (expected counter 32 in every row).

## Gate

`crates/core/tests/nrf/nrf_rtc_lfclk_clock.rs`

- `nrf_rtc_counts_lfclk_not_cpu_cycles` scans `configs/chips` and
  `configs/chips/onboarding` for every `nrf52840_rtc` / `nrf52_rtc` instance,
  builds the real `SystemBus`, and requires `COUNTER == cycles * 32768 /
  cpu_hz`. It collects every wrong instance in one panic.
- `nrf52840_rtc_rate_is_unchanged_at_64_mhz` is the byte-identical control:
  62 500 CPU cycles must still read 32.

Result: 2/2 pass in default and `event-scheduler` modes.

## Negative control

`negative-control.log` is the same binary with `Peripheral::attach_cpu_hz` on
`Nrf52Rtc` turned into a no-op (the pre-fix model). It names every wrong
instance:

```
nrf51822.yaml `rtc0`: after 15625 CPU cycles at 16000000 Hz the RTC reads 8, expected 32
nrf51822.yaml `rtc1`: after 15625 CPU cycles at 16000000 Hz the RTC reads 8, expected 32
nrf5340.yaml `rtc0`: after 125000 CPU cycles at 128000000 Hz the RTC reads 64, expected 32
nrf5340.yaml `rtc1`: after 125000 CPU cycles at 128000000 Hz the RTC reads 64, expected 32
```

The 64 MHz rows pass in the same run, so the gate is sensitive to the core
clock and not to the measurement itself.

## Real-Zephyr check

Unmodified upstream Zephyr v3.7 `hello_world` for
`nrf5340dk/nrf5340/cpuapp` still boots and prints its banner with the corrected
(half-speed) RTC:
`cargo test -p labwired-core --test gates firmware_survival::test_nrf5340_zephyr_survival`
-> 1 passed.

## Sources

- nRF51822 PS v3.1 §10: RTC counts 32.768 kHz LFCLK on a 16 MHz Cortex-M0.
- nRF52840 PS v1.7 §6.21 and nRF5340 PS v1.5 §6.21: same LFCLK source, cores at
  64 MHz and 128 MHz.
- Zephyr `nrf_rtc_timer` divides the same 32.768 kHz LFCLK by `PRESCALER`, which
  is why a wrong ratio moves every kernel tick.
