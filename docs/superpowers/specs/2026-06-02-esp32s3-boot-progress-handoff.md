# ESP32-S3 PlatformIO-in-sim — boot progress & handoff (2026-06-02)

Resumable state for the closed-loop goal: run a PlatformIO-built ESP32-S3
Arduino+Unity firmware inside the LabWired sim until it prints Unity results.

## Branches / artifacts
- Core engine: `labwired-core` worktree `feat/esp32s3-platformio`
  (`~/Projects/labwired/core/.worktrees/esp32s3-platformio`).
- Superrepo: `feat/platformio-closed-loop`
  (`~/Projects/labwired/.worktrees/platformio-closed-loop`).
- Firmware project: `integrations/platformio/esp32s3-unity-demo`
  (4 MB / DIO / default.csv; builds `firmware.elf`, entry `0x40379aac`).
- Repro: `bash /tmp/lw_diag.sh <steps>` (runs the ELF in the sim, resolves the
  crash PC to a symbol). Sim binary: the worktree's `target/release/labwired`.
  Chip: `core/configs/chips/esp32s3-zero.yaml`.

## Core commits this session (in order)
1. `cb2a128` map RTC slow/fast mem + ROM libc (memset/memmove/memcmp)
2. `d975243` ROM cache API nops + EXTMEM DCache-state busy-wait model
3. `c699cb9` `_xtos` critical-section thunks + regi2c mask + cache-freeze RMW
4. `c7d6d1c` SPIMEM1 flash-command stub (CMD auto-clear) + libgcc ROM thunks
5. `8cd95f9` **SALT/SALTU** instruction (decoder + interpreter + decode test) + eFuse getters
6. `741d272` pre-paint ESP-IDF dual-core handshake flags in the `run` path

## Boot-advance chain (each wall found by PC→symbol, then fixed)
```
load-reject(RTC mem) → memset → ROM cache API → Cache_Suspend_DCache spin →
_xtos_set_intlevel → regi2c_*_mask → Cache_Freeze_{I,D}Cache spins →
bootloader_flash_execute_command_common (SPI-flash CMD poll) → libgcc tail →
✦ APPLICATION code ✦ → SALTU (s_get_bus_mask) → eFuse getter cluster →
dual-core handshake → system_early_init spin  ← CURRENT WALL
```
The sim now executes the **entire ESP-IDF boot + ~30M instructions of app init**.

## Current wall (start here)
`system_early_init+0xc1`, tight spin around PC `0x42007d95`–`0x42007db0`:
reads two bytes `[a6+0]`/`[a6+1]`, ANDs them, calls `esp_rom_delay_us(100)`,
loops while the AND == 0 (`beqz a7, 0x42007d95`). The dual-core handshake flags
are now pre-painted and that did **not** break this spin, so the poll is on a
different condition. `a6` source is ambiguous in static disasm
(`esp_log_default_level @0x3fc97044` vs `s_cpu_up @0x3fc9add7`) — **first task:
confirm `a6` at the loop** (single-step under the sim's gdbstub, or add a
temporary trace of the polled address) and identify what sets it on silicon.

## PROPER MODEL pivot (the right architecture — supersedes thunks)

The thunk approach (faking ROM fns, pre-painting the handshake, nop-ing cache)
**diverges from silicon by construction** and is the wrong path for a fidelity-
first simulator. The proper model runs the chip's *real* ROM + real peripherals
so the firmware executes the identical instruction stream to silicon. This is
the core#105 chip-model roadmap (BROM/TIMG/flash/Wi-Fi).

Done this session toward it:
- **Dumped the real ROM/DROM off the chip over JTAG** (`esp32s3_rom_dump/`:
  `irom_40000000.bin` 384KB, `drom_3ff00000.bin` 128KB).
- **Wired real ROM/DROM into the sim** (env-gated `LABWIRED_ESP32S3_ROM` /
  `_DROM`; `RamPeripheral::with_image`; thunks disabled when set).
- **Verified real ROM executes** — the firmware ran real `memset` and the real
  `Cache_Disable_ICache` body (`0x4004f2b8`) with zero thunks.
- **Finding:** real ROM fns then read a RAM-resident pointer/hook table that
  only the ROM bootloader populates → jump-to-0. So faithful ROM execution
  requires running the **BROM from the reset vector (0x40000400)**, which needs
  the **SPI-flash controller** modeled (so the ROM reads the app image).

Proper-model phases (each diff-verifiable against silicon via the JTAG oracle):
1. **BROM phase** — boot from 0x40000400 on the real ROM; model the SPI-flash
   controller (real flash reads) so the 2nd-stage bootloader + app load. Deletes
   ALL ROM-fn thunks + the SPIMEM1 CMD hack.
2. **Timer/interrupt phase** — systimer + interrupt matrix → real FreeRTOS tick.
3. **SMP phase** — run both Xtensa cores → handshake happens naturally (deletes
   the handshake pre-paint).
4. Then `setup()` → Unity, byte-compared to the captured HW oracle.

Keep from this session (faithful): SALT/SALTU (silicon-verified), RTC RAM,
the real EXTMEM/SPIMEM register semantics. Delete once phase 1 lands: the ROM
thunks, handshake pre-paint.

## Remaining plan to green Unity (multi-session)
1. Pin the `system_early_init` poll target; satisfy it (model the writer:
   likely the **APP_CPU** or a **timer/ISR**).
2. **The scheduler boss**: model the second Xtensa CPU and/or the **systimer
   tick interrupt** driving FreeRTOS `vTaskSwitchContext`, so `loopTask` is
   dispatched and `setup()` runs. (The `run` path is single-CPU today; the
   `Machine` type already supports `cpu_secondary`, used by snapshot-capture.)
3. `setup()` → Unity → `UNITY_END` prints `N Tests M Failures` over serial.
   For the sim to *capture* it, route Unity to USB-Serial-JTAG (the sim streams
   that to stdout) OR read the `Unity` result struct from RAM.

## Hardware oracle — recipe + recovery (IMPORTANT)

JTAG halt/read is validated (`tool-openocd-esp32`, `board/esp32s3-builtin.cfg`):
`init; halt; reg pc; mdw <addr>` works and the link survives a multi-second run.

**Hard-won facts about this host + the S3 native USB:**
1. esptool's "Hard resetting via RTS pin" is a **no-op** on the native USB S3
   (no RTS line to EN) — the chip stays in the ROM download stub (PC parks at
   `0x40041A76`) and **never boots the app**. This is why JTAG always found it
   in ROM with the `Unity` struct uninitialized.
2. **`esptool --after watchdog-reset`** triggers a full RTC reset that DOES boot
   the flash app, no button needed. CONFIRMED (the app booted + re-enumerated).
3. **NEVER flash the HWCDC firmware** (`ARDUINO_USB_CDC_ON_BOOT=1`) on this host:
   the Arduino HWCDC re-enumerates USB on boot and the device never comes back —
   the whole `303a` drops off the bus, making the board **remotely
   unrecoverable** (no port to flash/JTAG). Recover only by a physical
   BOOT+power-cycle into download mode. (This happened 2026-06-02; board is
   currently in this state, awaiting physical recovery.)

**The working oracle recipe (no buttons), once the board is reachable:**
1. Build/flash the **no-CDC** firmware (current `platformio.ini`; JTAG-stable —
   IDF console stays on the ROM USB-Serial-JTAG, no re-enumeration).
2. `esptool --port /dev/ttyACM* --after watchdog-reset flash-id` → boots the app.
3. Wait ~3 s (app boot + `delay(2000)` + Unity).
4. `openocd -f board/esp32s3-builtin.cfg -c "init" -c "halt" -c "mdw 0x3fc9a788 8"`
   — read the `Unity` struct: NumberOfTests@+12 (expect 2), TestFailures@+16
   (expect 0). CAVEAT: confirm OpenOCD `init` doesn't reset the chip back to ROM
   (it may); if it does, attach without reset or breakpoint `UnityEnd`
   (`0x42001fbc`) before the watchdog boot.
   `Unity` @ `0x3fc9a788`, `UnityEnd` @ `0x42001fbc` (current no-CDC build).

Output routing is irrelevant for the JTAG-RAM read — the `Unity` struct is
populated whether Unity prints to USB-CDC or UART0.

**Recovery for the current stuck state:** physically hold BOOT + power-cycle (or
BOOT+tap EN) to enter download mode, then reflash the no-CDC firmware.

## Also parked
- Reply to Dr. Ivan Kravets (PlatformIO) — draft ready, awaiting send decision.
- Spec review of `2026-06-02-platformio-closed-loop-design.md`.
- Seam A: wire `test_testing_command` → `labwired` (stop-on-Unity-summary +
  exit codes) — demonstrable now on a board that already runs in the sim
  (e.g. a Cortex-M Unity ELF), independent of the S3 scheduler.
