# nRF54L15 RRAMC write gate; Zephyr v4.4.0 `soc_flash_nrf` runs (2026-11-28)

## Result

Unmodified Zephyr v4.4.0 `samples/drivers/soc_flash_nrf`, built for
`nrf54l15dk/nrf54l15/cpuapp` with the default configuration, now runs to
`Finished!` on the `nrf54l15` profile. Before this change the same image
printed nothing. Defects 1 and 2 stopped the image. Defect 3 did not stop
it, but the RRAMC status registers and the WEN gate were wrong.

## Defects

1. **CLOCK IRQ lost when INTENSET follows the event.**
   nrfx `nrfx_clock_lfclk_start` writes `TASKS_LFCLKSTART` and then
   `INTENSET`. The model latched `EVENTS_LFCLKSTARTED` before the `INTENSET`
   write and raised the IRQ only when an event settled. The IRQ line is a
   level (`INTEN & EVENTS`), so the NVIC never saw it. Zephyr
   `lfclk_spinwait` then slept in `WFE` forever (`arch_cpu_atomic_idle`,
   PC 0x4E18).
   Measured at the stall: `EVENTS_LFCLKSTARTED = 1`, `INTEN = 4`,
   `INTPEND = 4`, `NVIC ISPR8 = 0`, `ISER8 = 0x2020`, `SCR = 0x10`.
2. **SPIM00 not mapped.** The DK devicetree enables `spi00` for the on-board
   MX25R64 SPI-NOR. An image built with `CONFIG_FLASH` applies the SPIM00
   pinctrl at boot. The store to `PSEL` at 0x5004A600 took a precise
   BusFault (`BFAR 0x5004a600`, PC 0x225C, `pinctrl_configure_pins`).
3. **RRAMC was a zero-filled window.** `READY` and `READYNEXT` read 0
   ("RRAMC is busy"), `BUFSTATUS.WRITEBUFEMPTY` read 0 ("data needs
   committing") and a CPU store reached the RRAM array with `CONFIG.WEN`
   clear.

## Fix

- `crates/core/src/peripherals/nrf54l/clock.rs`: an `INTEN`/`INTENSET` write
  that enables a source with a latched event schedules one IRQ-line
  re-evaluation (legacy tick and scheduler event).
- `configs/chips/nrf54l15.yaml`: `spi00`, type `nrf54l_spim`, 0x5004A000,
  IRQ 74 (SVD `GLOBAL_SPIM00_S`). No SPI-NOR device is attached.
- New model `nrf54l_rramc` (`crates/core/src/peripherals/nrf54l/rramc.rs`)
  for nRF54L15. It uses the existing generic flash-store owner hook
  (`owns_flash_stores` / `flash_cpu_store`); no bus change.
  - `CONFIG.WEN` clear: a store into RRAM is dropped.
  - `CONFIG.WEN` set: a byte, half-word or word store commits as written.
    RRAM writes 0 and 1 alike; there is no erase.
  - `READY` and `READYNEXT` read 1. `EVENTS_READY` latches when a buffer
    commit completes, and on every store when `WRITEBUFSIZE` is Unbuffered.
  - `BUFSTATUS.WRITEBUFEMPTY` reads NotEmpty from the first buffered store
    until the last byte of the buffer area is written, a store lands outside
    the buffer area, or `TASKS_COMMITWRITEBUF` is triggered.
  - Other registers hold the SVD reset values and field masks.

Sources: Nordic `nrf54l15.svd` (`GLOBAL_RRAMC_S`); product specification
RRAMC sections "Writing to RRAM", "Buffered RRAM write" and "Erasing RRAM"
(text read from the nRF54LM20A edition; the nRF54L15 SVD has the same
fields); nrfx `nrf_rramc.h`; Zephyr `drivers/flash/soc_flash_nrf_rram.c`.

## Evidence

Fixture `tests/fixtures/nrf54l15-zephyr-soc-flash.elf`
(sha256 `b5de86048cb3e2a014ab76e792afe4c8cf7f95c3f31ccab1ad1869a9b0d692b4`):

```
cd ~/zephyrproject-v4.4-labwired-test        # zephyr v4.4.0 684c9e8f32e, hal_nordic 44fd3d4
ZEPHYR_TOOLCHAIN_VARIANT=gnuarmemb GNUARMEMB_TOOLCHAIN_PATH=/usr \
  west build -p always -b nrf54l15dk/nrf54l15/cpuapp zephyr/samples/drivers/soc_flash_nrf
arm-none-eabi-strip --strip-debug zephyr.elf   # arm-none-eabi-gcc 13.2.1
```

A second build to confirm byte-identical output was not run.

Gate `crates/core/tests/nrf/nrf54l15_rramc.rs` (4 tests):

- `idle_rramc_reads_ready_not_busy`
- `wen_gates_rram_stores_and_overwrite_needs_no_erase`
- `buffered_store_reports_uncommitted_data_until_committed`
- `zephyr_soc_flash_sample_writes_and_reads_back_rram`: the UART shows eight
  `Writing 16 data bytes` lines, one `Data read matches data written. Good!`
  and `Finished!`; RRAM 0x174000..0x17407F holds bytes 0..127; `CONFIG.WEN`
  is clear and the write-buffer is empty at the end.

Model unit tests: `peripherals::nrf54l::rramc` 7/7,
`nrf54l::clock::tests::intenset_after_a_latched_event_raises_the_irq`, and
under `event-scheduler`
`nrf54l::clock::tests::intenset_after_a_delivered_event_schedules_a_line_evaluation`.

Negative controls (measured on this branch):

| Change reverted | Result |
|---|---|
| CLOCK fix | No UART output. CPU asleep at PC 0x4E18 after 10,000,000 steps. |
| `spi00` entry (CLOCK fix present) | BusFault, `BFAR 0x5004a600`, before the sample banner. |
| `nrf54l_rramc` -> `nrf54l_rramc_stub` (other fixes present) | 4/4 tests fail. Log: `negative-control-rramc-stub.log`. |

The firmware UART lines alone do not separate the RRAMC model from the old
window: with the stub the sample still prints `Finished!`, because the stub
let every store through. The firmware test fails on the stub only at the
final `WRITEBUFEMPTY == 1` check. The WEN gate is proven by the bus tests.

## Limits

- No write latency: `READY` is never observed as Busy.
- The write-buffer is state only. Data is in the array at the store. The
  commit on a CPU read of a buffered line and the `READYNEXTTIMEOUT` commit
  are not modelled. The buffer area is assumed to start at the first
  buffered 128-bit line.
- A store with `WEN` clear is dropped without a BusFault or `ACCESSERROR`.
  The sources above do not state what silicon does.
- `ERASE.ERASEALL` is not modelled: a write of `Erase` fails the access.
- `REGION[n]` protection, POWER modes, ECC, DPPI and the IRQ line (75) are
  not modelled. UICR remains a zero-filled window and is not written through
  RRAMC.
- nRF54LM20A keeps `nrf54l_rramc_stub`: its RRAMC adds `TASKS_CLRWRITEBUF`,
  which needs a staged buffer this model does not have.
- No nRF54L15 silicon capture. nRF54L15 is not one of the 245 imported
  Renode platforms; the onboarding scoreboard does not change.
