# PlatformIO × LabWired — Closed-Loop Firmware Verification (ESP32-S3)

**Date:** 2026-06-02
**Branches:** superrepo `feat/platformio-closed-loop`, core `feat/esp32s3-platformio`
**Status:** design — grounded in an empirical sim divergence map (below)

## The story

An AI agent **authors and compiles firmware in PlatformIO**, then **deploys
that build into the LabWired simulator** to run and verify it — no board in
the loop. PlatformIO is the author+build half; LabWired is the deterministic
run+verify half. Together they close the agent dev-cycle. **Hardware parity**
(sim serial output == real silicon serial output) is what makes the loop
trustworthy.

The seam is exactly PlatformIO's documented simulator hook — `test_testing_command`
in `platformio.ini`, the same mechanism the Renode integration uses. PlatformIO
builds `firmware.elf`; instead of flashing hardware it hands the ELF to
`labwired`, which runs headless, streams serial to stdout, and exits when the
Unity summary line appears. PlatformIO parses pass/fail from that stdout.

First target: **ESP32-S3** (the board physically connected on `/dev/ttyACM1`,
VID:PID `303a:1001`), validated against real silicon.

## Architecture

Two components with a clean interface.

```
   ┌─────────────── PlatformIO (author + build) ───────────────┐
   │  agent writes test  →  pio test  →  firmware.elf          │
   └────────────────────────────┬──────────────────────────────┘
                                 │  test_testing_command  (Component A)
                                 ▼
   ┌─────────────── LabWired (run + verify) ───────────────────┐
   │  labwired runs ELF on the esp32s3 model  (Component B)    │
   │  → streams USB-Serial-JTAG to stdout                      │
   │  → stops on Unity summary, maps pass/fail to exit code    │
   └────────────────────────────┬──────────────────────────────┘
                                 │  same ELF
                                 ▼
            real ESP32-S3 (parity oracle: serial diff)
```

- **Component A — the seam (board-agnostic).** A `labwired` run mode suitable
  as `test_testing_command`: load ELF, run headless, echo serial to stdout,
  **stop the instant Unity prints `N Tests M Failures K Ignored`**, exit `0`
  on all-pass / non-zero otherwise. Plus the example PlatformIO project and
  docs. *Independently testable; the spine of the story.*
- **Component B — ESP32-S3 execution (the hard part).** Make a full
  ESP-IDF/Arduino image boot past `app_main` → FreeRTOS scheduler → first task
  → `setup()` → Unity output. This is the core#105-class work.

**Interface:** A consumes "a runnable ELF that emits Unity output to serial";
B produces "an S3 ELF that runs far enough to do so." Green `pio test` on S3
lands when both are done. A can be exercised today against any board the sim
already runs (e.g. a Cortex-M Unity ELF) to de-risk the seam in parallel.

## Empirical gap map (measured 2026-06-02)

Built a real Arduino+Unity image with PlatformIO
(`integrations/platformio/esp32s3-unity-demo`, `firmware.elf` 6.9 MB,
entry `0x40379ba0`) and ran it through `labwired run` against
`esp32s3-zero.yaml`. Divergence is layered:

### Layer 1 — Memory map  ✅ DONE (core commit `cb2a128`)
ELF has 8 LOAD segments; two landed in unmapped space and the loader refused
them:
- `0x50000200` → **RTC SLOW RAM** (`0x50000000`, 8 KiB) — `RTC_DATA_ATTR`.
- `0x600FFFD8` → **RTC FAST RAM** (`0x600FE000`, 8 KiB) — `RTC_NOINIT`/wake stub.

Both added as plain `RamPeripheral`s in `configure_xtensa_esp32s3`. Result:
ELF loads, CPU executes from entry.

### Layer 2 — ROM thunk tail  ◑ IN PROGRESS
The S3 `run` path registered only the 14 thunks a minimal **esp-hal** boot
needs. A full ESP-IDF/Arduino image calls more ROM functions (the on-chip ROM
is not hosted, so unstubbed calls hit zeroed bytes → `IllegalInstruction`,
EXCCAUSE 0). Walked so far:
- **`memset` `0x400011e8`** (sibling of stubbed `memcpy`) → registered the
  existing `rom_memset`; also wired `memmove 0x40001200`, `memcmp 0x4000120c`. ✅
- Next wall: **`Cache_Disable_ICache` `0x4000186c`** — the ROM
  cache-management family (`Cache_Disable/Enable_ICache`,
  `Cache_Set_IDROM_MMU_Size/Info`, freeze/occupy, `0x4000186c`–`0x400019bc`).
  All nop-able: flash-XIP is identity-mapped in the model, so cache ops are
  no-ops. ☐

This layer is bounded and mechanical. **The `run_snapshot_capture` path
already installs a large, hand-validated ESP-IDF/Arduino thunk set** — the
plan is to factor that out and reuse it, not rediscover thunks by
crash-iteration.

### Layer 3 — ESP-IDF startup + FreeRTOS scheduler  ☐ THE DEEP WALL
ESP-IDF startup and FreeRTOS execute from the ELF's own (loaded) flash/IRAM —
no ROM hosting needed. The genuine blocker is the **Xtensa context-switch**:
building a task's initial stack frame and `rfi`/`rfe`-ing into it, plus the
timer-tick interrupt path. This is the known core#105 problem; the
snapshot-capture path invests heavily here and still stalls at first task
dispatch. This layer is research, not wiring, and is the bulk of the effort.

## Implementation plan (staged; parity-checked at each layer)

Establish the **HW oracle first**: flash `firmware.elf` to the real S3
(`/dev/ttyACM1`; manual BOOT+RESET to enter download mode — the native
USB-Serial-JTAG hits an `Errno 71` RTS quirk on auto-reset), capture genuine
Unity serial. That is the ground truth every sim stage is diffed against.

1. **Layer 1 — memory map.** ✅ committed.
2. **Layer 2 — ROM bring-up.** Factor `run_snapshot_capture`'s ESP-IDF/Arduino
   thunk installation into a shared helper; apply it on the `run` path. Add
   the cache-management family. Advance boot to the scheduler.
3. **Layer 3 — scheduler/context-switch.** Model the FreeRTOS-Xtensa context
   restore + timer tick until `setup()` runs and Unity prints. Diff serial vs.
   oracle after every increment.
4. **Component A — the seam.** Add a `--stop-on-regex`/Unity-summary stop
   condition and Unity-aware exit codes to the run/test mode; wire
   `test_testing_command` in `platformio.ini`; `pio test` → green.
5. **Parity gate + docs.** Lock a CI check asserting sim serial == captured
   silicon serial for this firmware. Write the integration doc; prep the
   reply/demo for PlatformIO.

## Testing strategy

- **Parity oracle:** byte/line diff of Unity output, sim vs. real S3, same ELF.
- **Regression:** the existing esp-hal hello-world S3 path must still pass
  (Layer-1/2 changes are additive — new RAM regions + new thunk addresses).
- **Seam unit test:** feed a canned Unity-summary stream to Component A; assert
  stop timing and exit code.
- **End-to-end:** `pio test` exits 0 on all-pass, non-zero on a deliberately
  failing assertion.

## Scope / YAGNI

- One board (ESP32-S3), one framework path (Arduino), one minimal Unity test.
- No WiFi/BLE ROM (linked but never called on the boot→`setup()` path).
- No multi-core scheduling beyond what first-task dispatch requires.
- Component A stays board-agnostic but is only *shipped* wired for S3.

## Risks

- **Layer 3 is open-ended.** Context-switch modeling may surface further
  Xtensa-HAL gaps (window spill on exception entry). Mitigation: HW oracle
  pinpoints each divergence; reuse JIT `windowed_call` machinery.
- **ROM tail longer than measured.** Mitigation: reuse snapshot-capture's
  proven thunk set wholesale.
- **USB-JTAG flashing quirk** (`Errno 71`). Mitigation: manual BOOT+RESET;
  document it; J-Link/OpenOCD fallback exists on this host.
