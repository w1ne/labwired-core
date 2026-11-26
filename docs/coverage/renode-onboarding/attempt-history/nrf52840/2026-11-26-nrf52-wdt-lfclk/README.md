# nRF52 WDT counts LFCLK ticks (2026-11-26)

## Defect

`Nrf52Wdt` decremented CRV once per CPU cycle. Silicon counts CRV in
32.768 kHz LFCLK ticks: the timeout is `(CRV + 1) / 32768` s (nRF52840 PS
v1.7 §6.34.1). On a 64 MHz part every watchdog fired about 1953x early; a
1 s Zephyr watchdog (`CRV = 32767`) fired after 32 767 CPU cycles, about
0.5 ms. On the 16 MHz nRF51822 the error was about 488x.

## Fix

The model converts CPU cycles into LFCLK ticks with an exact rational
accumulator (`cycles * 32768 / cpu_hz`, u128 intermediate). The core clock
comes from `Peripheral::attach_cpu_hz` (descriptor `cpu_hz`); the default
is 64 MHz. The counter is loaded with `CRV + 1` ticks on start and on
reload. The scheduler-mode wake delay uses the same conversion.

## Evidence

Gate `crates/core/tests/nrf/nrf_wdt_lfclk_clock.rs` scans `configs/chips`
and `configs/chips/onboarding`, programs `CRV = 32` on every instance and
requires EVENTS_TIMEOUT clear at `ceil(33 * cpu_hz / 32768) - 1` cycles and
set at `ceil(33 * cpu_hz / 32768)` cycles.

| Descriptor | cpu_hz | Timeout cycle |
| --- | --- | --- |
| nrf51822 | 16 MHz | 16 114 |
| nrf52832, nrf52833, nrf52840 | 64 MHz | 64 454 |
| onboarding/arduino_nano_33_ble, onboarding/nrf52840, onboarding/nrf52840dk_nrf52840 | 64 MHz | 64 454 |

- New model: gate 2/2 pass, default and `event-scheduler`.
- Negative control (model from `origin/main` 21d1f5f37f): gate fails on all
  7 instances; EVENTS_TIMEOUT is already set one cycle early. Log:
  `negative-control.log`.

## Limits

- The model still does not reset the CPU on timeout. It sets
  EVENTS_TIMEOUT and pends the IRQ only.
- CONFIG.SLEEP / CONFIG.HALT pause behaviour is not modelled.

## Review follow-up

Independent review (in-session subagent): APPROVE WITH NITS. Actioned:

- Scheduler path now tested: `on_event` at 64 MHz after a mid-period
  RR reload bites on cycle 93 750 exactly, ignores a stale token.
- RREN and CONFIG reset to 0x1 (PS §6.34.9), not 0.
- TASKS_START while running no longer reloads the counter.

Unit tests 17/17, default and `event-scheduler`.
- nRF54L WDT30/WDT31 remain `nrf54l_wdt_stub` windows.
