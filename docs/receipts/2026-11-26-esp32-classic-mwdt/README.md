# ESP32 (classic): the TIMG main-system watchdog actually times out

**Demand rank:** the ESP32 family is **#1** in the JetBrains State of Embedded
2025 MCU survey (38 % of respondents; 43 % among enthusiasts). This is depth
work on a part the engine already boots, not a new part: the gap was that the
classic TIMG watchdog accepted every feed and never latched, so firmware that
starved it ran to completion and the run looked green.

**Board:** Espressif ESP32-DevKitM-1 / ESP32-MINI-1 (classic ESP32-WROOM class
silicon, `configs/chips/esp32.yaml`), console on UART0.

**Vendor status, checked live 2026-11-26** (the part is not NRND; Espressif's
longevity commitment covers ESP32):

| URL | HTTP | What it says |
|---|---|---|
| <https://documentation.espressif.com/esp-dev-kits/en/latest/esp32/esp32-devkitc/user_guide.html> | 200 | Current ESP32-DevKitC guide; names **ESP32-DevKitC-32E** and **ESP32-WROOM-32E** (single occurrence of the board name, six of the module). |
| <https://www.espressif.com/en/products/modules> | 200 | Current module catalogue. |
| <https://www.espressif.com/en/products/socs/esp32> | 200 | Current ESP32 SoC product page. |
| <https://www.espressif.com/en/products/longevity-commitment> | 200 | Longevity commitment; ESP32 listed for at least 15 years from 2016. |

Purchase links for ESP32-WROOM-32E-N4 (DigiKey, Mouser, Amazon US, Robu India)
were recorded in the 2026-11-09 deep-sleep receipt and the module catalogue
still lists them.

## Result

| fixture | before | after |
|---|---|---|
| `wdt_starve.elf` | never latches — `WDT_ARM` only, 2,000,000-cycle budget exhausted | `WDT_ARM` / `WDT_FIRED` / `WDT_CLEARED`, PASS 2/2 checks, **660,000 steps, 0.21 s** |
| `wdt_feed.elf` | `WDT_FED_OK` (passes without a watchdog model: "no timeout" proves nothing) | `WDT_FED_OK`, PASS 1/1 checks, **410,000 steps, 0.16 s** |

Both scripts run the CLI's default build, which reports
`event_scheduler=on` — the scheduled-wake path, not the legacy walk. The same
two fixtures also pass the integration test with the feature **off**
(`cargo test -p labwired-core --test esp esp32_classic_wdt::`), so both timing
paths are covered.

## What changed

1. **The classic register map was wrong.** `peripherals/esp32/timg.rs` placed
   WDTFEED at `0x5C` and WDTWPROTECT at `0x60` for the classic layout. Both
   `esp-idf v5.4 components/soc/esp32/register/soc/timer_group_reg.h` and the
   in-tree vendor SVD `tests/fixtures/real_world/esp32.svd` put
   **WDTCONFIG5 (STG3_HOLD) at 0x5C, WDTFEED at 0x60, WDTWPROTECT at 0x64**
   (six config words, not five). Both old offsets were no-ops, so nothing had
   observed the difference; the fixture now pins the vendor map
   (`classic_mwdt_register_map_matches_the_vendor_header`).
2. **The four-stage chain is real on classic ESP32** (`Timg::with_classic_mwdt`,
   wired for TIMG0 and TIMG1 in `peripherals/esp32/factory.rs`):
   - WDTCONFIG0.WDT_EN 0→1 arms stage 0; a write to WDTFEED restarts at stage 0;
     expiry advances `0 → 1 → 2 → 3 → 0` loading each stage's hold.
   - Holds are WDTCONFIG2..5, in APB cycles scaled by
     WDTCONFIG1.WDT_CLK_PRESCALE ([31:16]) — 400 APB cycles = 5 µs at the
     model's 80 MHz APB.
   - WDTWPROTECT holding anything but `WDT_WKEY` (0x50D8_3AA1) drops writes to
     WDTCONFIG0..5; WDTFEED and WDTWPROTECT stay writable while locked.
   - An interrupt-action stage (WDTCONFIG0 STG* = 1) sets
     **INT_RAW_TIMERS bit 2** — classic has T0 (0), T1 (1), WDT (2), LACT (3);
     the C3/C6 layout's WDT is bit 1 of its own 0x74 register, which is why the
     two layouts remain separate opt-ins. INT_CLR_TIMERS bit 2 is W1C.
3. **DPORT delivery.** A latched, enabled watchdog with
   `WDT_LEVEL_INT_EN`/`WDT_EDGE_INT_EN` set puts `base + 2`
   (`ETS_TIMERG0_WDT_INTR` = 16, `ETS_TIMERG1_WDT_INTR` = 20) on the interrupt
   matrix poll and on the scheduler's wake set, next to the existing T0
   (base+0), T1 (base+1) and LACT (base+3) sources.
4. **Timing stays on the existing seam.** The watchdog advances in `sync_to` /
   `tick_elapsed` with the same CPU→APB conversion as T0/T1 and is included in
   `next_alarm_delay`, so the scheduler wakes at the expiry even while firmware
   only reads `INT_RAW`. A feed deliberately does **not** schedule a new wake:
   the write already synced the model, and one wake per feed would pile events
   into `MAX_LIVE_EVENTS_PER_PERIPHERAL` (measured: the fed fixture tripped that
   ceiling before the fix).

## Evidence

Committed fixtures (`crates/core/tests/fixtures/esp32-classic-wdt/`):

- `wdt_starve.S` / `wdt_feed.S` — hand-written Xtensa, register level, no ROM,
  no IDF, masks maskable interrupts with `rsil` so the DPORT line is asserted
  without being taken.
- `build.sh` — rebuilds both ELFs with `xtensa-esp32-elf-gcc`
  (PlatformIO `toolchain-xtensa-esp-elf`); the ELFs are committed so CI needs no
  toolchain.
- `wdt_starve.test.yaml`, `wdt_feed.test.yaml` — CLI acceptance scripts.

Tests:

- `crates/core/src/peripherals/esp32/timg.rs` unit tests, 6 new:
  vendor register map and write lock, bit-2 latch + INT_ST gating + W1C clear,
  feed re-arm and disable, prescaler scaling, DPORT source base+2 for TIMG0 and
  TIMG1, and `uses_scheduler()` unchanged (existing classic labs keep the
  scheduler).
- `crates/core/tests/esp/esp32_classic_wdt.rs` — runs both ELFs on
  `configs/systems/esp32-devkitm-1.yaml` and asserts the exact console.

Logs: `after/wdt_starve.log`, `after/wdt_feed.log`,
`before/measurement.txt` (negative control, same fixtures, one-line revert).

## Not covered (and not claimed)

- **STG = 2 (reset CPU) and STG = 3 (reset system) do not reset.** The chain
  advances and no latch is taken, because no peripheral has a safe
  reset-request path on the bus. A chain that ends in a reset is modelled as a
  chain that silently wraps.
- **The edge/level interrupt distinction is not modelled**: either enable bit
  asserts a level on the matrix. `WDTCONFIG0` reset-length fields
  ([20:15]) and `WDT_FLASHBOOT_MOD_EN` are stored but have no effect.
- **The RTC_CNTL watchdog is a different block** (`RTC_WDTCONFIG0` at 0x8C,
  inside `rtc_cntl`) and remains unmodelled.
- **`docs/coverage/tier1-matrix.json` still records `esp32/wdt` as `na`.** A
  cell only moves when the tier-1 fixture (`tests/fixtures/tier1/esp32.elf`)
  prints `TIER1 wdt PASS`, and that fixture is untouched here; the evidence for
  this round is the PR-gated integration test and the two CLI scripts above.
  Raising the cell without changing that fixture would claim more than the
  protocol measured.
- No silicon capture: this is sim-consistent, not silicon-anchored (the
  repository's tier-1 convention). The register map is anchored to the vendor
  header and SVD quoted above.

## Reproduce

```sh
# Rebuild the fixtures (needs the Espressif Xtensa GCC)
crates/core/tests/fixtures/esp32-classic-wdt/build.sh

# Acceptance, from the repository root
labwired test --script crates/core/tests/fixtures/esp32-classic-wdt/wdt_starve.test.yaml
labwired test --script crates/core/tests/fixtures/esp32-classic-wdt/wdt_feed.test.yaml

# Rust gates, both timing paths
cargo test -p labwired-core --lib peripherals::esp32::timg
cargo test -p labwired-core --test esp esp32_classic_wdt::
cargo test -p labwired-core --features event-scheduler --test esp esp32_classic_wdt::
```
