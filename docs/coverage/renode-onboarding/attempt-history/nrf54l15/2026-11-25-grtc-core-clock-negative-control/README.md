# nRF54L GRTC counted a fixed 128-cycle SYSCOUNTER tick (2026-11-25)

`Nrf54lGrtc` converted CPU cycles into SYSCOUNTER (1 MHz) increments with the
compile-time constant `CPU_HZ_DEFAULT / SYSCOUNTER_HZ == 128`. That is correct
only on the 128 MHz application core. On the 64 MHz pre-PLL boot state the
counter ran 2x slow (half rate); on a 16 MHz core it ran 8x slow. The error is
silent: a Zephyr tick still arrives, just at the wrong instant.

| core clock | SYSCOUNTER ticks after 1 ms before | after |
|---:|---:|---:|
| 128 MHz | 1000 | 1000 (unchanged) |
| 64 MHz | 500 | 1000 |
| 32 MHz | 250 | 1000 |

Measured with `MODE.SYSCOUNTEREN`, one millisecond of simulated time,
`SYSCOUNTER[0].SYSCOUNTERL` read at `0x500E2000 + 0x720`.

## Fix

`Nrf54lGrtc` now derives the ratio from `Peripheral::attach_cpu_hz`, which the
bus fills from `ChipDescriptor::cpu_hz` at every `add_peripheral` /
`upsert_peripheral` choke point. The fractional phase is kept in
`cycles * SYSCOUNTER_HZ` units (u64 remainder, u128 intermediate), so a core
clock that is not a whole multiple of 1 MHz still keeps exact time. The
descriptor's `cpu_hz` is the only source of the core clock; the 128 MHz
constant remains only as the hand-built-bus default.

At 128 MHz the behaviour is identical by construction, not merely by test:
`128e6 = 128 * 1e6`, so the remainder is the old integer remainder scaled by
1e6, and the compare-distance ceil reduces to the old `increments * 128 -
accum`. A clock change after time has advanced re-anchors the lazy clock at
"now" so already-counted cycles are not replayed at the new rate.

## Gate

`crates/core/tests/nrf/nrf_grtc_clock.rs` scans every chip descriptor that
declares `nrf54l_grtc`, assembles the real descriptors, and rewrites only
`cpu_hz` on `nrf54l15` to the clocks that matter:

- `every_nrf54l_grtc_counts_one_megahertz_at_the_descriptor_clock` — every
  shipped descriptor that declares a GRTC reaches 1000 ticks in 1 ms at its own
  `cpu_hz` (`nrf54l15`, `nrf54lm20a`).
- `nrf54l_grtc_follows_the_descriptor_core_clock` — the same hardware at
  128 / 64 / 32 MHz reaches 1000 ticks in 1 ms.

Result: 2/2 pass in default and `event-scheduler` modes. Unit tests
`peripherals::nrf54l::grtc::tests::core_clock` add the fractional 2.5 MHz
ratio, the zero-clock default, the no-op default attach and the armed-compare
deadline at 16 MHz: 5/5 pass. `peripherals::nrf54l::grtc::tests::scheduler_mode`
adds the mid-run clock change: 4/4 pass with `event-scheduler`.

## Negative control

`negative-control.log` is the same tree with `Nrf54lGrtc::attach_cpu_hz` turned
into a no-op. The 128 MHz row passes in the same run, so the gate is sensitive
to the core clock rather than to the measurement:

```
nrf54l15 GRTC at 0x500e2000 with cpu_hz=64000000: expected 1000 SYSCOUNTER
ticks per millisecond, got 500
core clock 64000000 Hz must keep the SYSCOUNTER at 1 MHz
```

## Sources

- nrfx `hal/nrf_grtc.h`:
  `NRF_GRTC_SYSCOUNTER_MAIN_FREQUENCY_HZ 1000000UL`.
- nRF54L15 application core: 128 MHz after PLL (`configs/chips/nrf54l15.yaml`);
  the part boots at 64 MHz before `PLL.FREQ` is raised (datasheet v1.0 §5.5.3).
- nRF54L HFXO: 32 MHz; PCLK16M: 16 MHz
  (`nrf54l15_application_peripherals.h`, Zephyr `nrf54l_05_10_15.dtsi`).
