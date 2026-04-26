# Case Study: ESP32-S3 Plan 2 — Boot Path + USB_SERIAL_JTAG + SYSTIMER

**Date closed:** 2026-04-26
**Branch:** `feature/esp32s3-plan2-boot-uart`
**Spec:** `docs/superpowers/specs/2026-04-26-plan-2-boot-uart-systimer.md`
**Implementation plan:** `docs/superpowers/plans/2026-04-26-plan-2-boot-uart-systimer.md`
**Milestone closed:** M3 (Fast-path boot reaches `main`. `hello-world` prints via USB_SERIAL_JTAG) from the ESP32-S3-Zero design spec.

---

## What Plan 2 Delivered

A real `esp-hal` Rust binary (`xtensa-esp32s3-none-elf` target) now runs end-to-end in the LabWired simulator. The same `examples/esp32s3-hello-world` ELF that flashes onto a connected ESP32-S3-Zero also boots in the sim and prints `Hello world!` to host stdout via USB_SERIAL_JTAG, paced by SYSTIMER through `esp_hal::Delay::delay_millis(1000)`.

```
$ labwired-cli run --chip configs/chips/esp32s3-zero.yaml \
                   --firmware …/esp32s3-hello-world
labwired-cli run: entry=0x40378e20 stack=0x3fcdb700 segments=5
Hello world!
Hello world!
…
```

### Test counts (final state)

| Suite | Passing | Notes |
|---|---|---|
| `labwired-core` (unit + integration) | 568 | +18 from Plan 1 close, mostly new peripheral tests |
| `labwired-core --features esp32s3-fixtures` | +1 | `e2e_hello_world` runs the actual firmware in CI |
| Total (excluding cross-compile crates) | 568+ | |

### Components shipped

| Component | File | LoC (approx) |
|---|---|---|
| Boot module + fast_boot (with XIP routing) | `crates/core/src/boot/{mod,esp32s3}.rs` | ~290 |
| ROM thunk dispatch + 14 default thunks | `crates/core/src/peripherals/esp32s3/rom_thunks.rs` | ~440 |
| BREAK 1,14 dispatch hook | `crates/core/src/cpu/xtensa_lx7.rs` (modified) | +30 |
| RSR/WSR/XSR/RUR/WUR + RSIL + EXTUI + MIN/MAX | `crates/core/src/{cpu,decoder}/xtensa*.rs` | +200 |
| UsbSerialJtag | `crates/core/src/peripherals/esp32s3/usb_serial_jtag.rs` | ~160 |
| Systimer (with VALUE_VALID bit) | `crates/core/src/peripherals/esp32s3/systimer.rs` | ~250 |
| System / RTC_CNTL / EFUSE stubs (with low/high MMIO catch-all) | `crates/core/src/peripherals/esp32s3/system_stub.rs` | ~250 |
| FlashXipPeripheral (per-window backings) | `crates/core/src/peripherals/esp32s3/flash_xip.rs` | ~170 |
| System glue + chip YAML | `crates/core/src/system/xtensa.rs`, `configs/chips/esp32s3-zero.yaml` | ~350 |
| Example firmware | `examples/esp32s3-hello-world/` | ~150 |
| CLI run subcommand (with PC ring buffer) | `crates/cli/src/main.rs` (modified) | +130 |
| E2E test | `crates/core/tests/e2e_hello_world.rs` | ~100 |
| Total | | ≈2,500 |

---

## Plan Corrections Caught During Implementation

These are the meaningful discoveries — issues where the plan's assumption was wrong and we had to fix the simulator (not just add stubs).

| # | Issue | Resolution |
|---|---|---|
| 1 | **CALL formula off-by-4 at 4-aligned PCs.** Plan 1's `CALL{0,4,8,12}` exec used `(PC+3) & ~3` for the target base, but the ISA RM specifies `(PC+4) & ~3`. The unit-test encoder used the same wrong base, so encode/decode round-tripped self-consistently — the bug was masked because there was no end-to-end firmware test. | Fixed to `(PC+4) & ~3`; re-derived 4 hw-oracle CALL test byte sequences. |
| 2 | **ENTRY exec read post-rotation register.** Plan 1's ENTRY used `AR[WB_new*4 + as]` for the SP source; the ISA RM says `AR[WB_old*4 + as]` (caller's register). With the wrong source, every chained CALL4 + ENTRY in real firmware computed `a1 = 0 - imm*8 = 0xFFFFFFC0` because the post-rotation slot was uninitialized. | Fixed; existing ENTRY tests updated to ISA-correct expectations. |
| 3 | **XSR decoder under wrong op1.** XSR was grouped with RSR/WSR at op1=3; actual encoding is op1=1, op2=6. `xsr.intenable` decoded to Unknown and trapped immediately. | Decoder fixed; verified by re-assembling all four with `xtensa-esp32s3-elf-as`. |
| 4 | **Flash-XIP backing aliasing.** Both DROM (`.rodata` at 0x3C00_0020) and IROM (`.text` at 0x4200_0020) shared a single `Arc<Mutex<Vec<u8>>>` backing buffer. Window-relative offset 0x20 collided — `.text` overwrote `.rodata`. The first jx-via-rodata-table read garbage. | Per-window backings (matches real silicon: shared SPI flash but independent MMU page tables per cache window). `BootOpts` now carries `icache_backing` and `dcache_backing` separately. |
| 5 | **`SystemBus::new()` seeds STM32 peripherals at the BROM range.** tim3 was registered at 0x4000_0400, shadowing every BROM thunk in 0x4000_0400…0x4000_07FF (e.g. `rtc_get_reset_reason` at 0x4000_057C). | `configure_xtensa_esp32s3` clears the seeded peripherals before installing the ESP32-S3 bank. |
| 6 | **`RomThunkBank::return_with` only handled CALL0.** Read a0 unmasked, ignored PS.CALLINC. Real BROM thunks reached via CALLX4/8 needed post-rotation a2 with bits[31:30] segment-bit preservation. | Switch on PS.CALLINC; mask bits[31:30]; OR in thunk PC's segment bits the way RETW does. |
| 7 | **Missing instructions: RSIL, EXTUI, MIN/MAX/MINU/MAXU.** Each surfaced after earlier blockers were resolved. RSIL is atomic-PS-read+INTLEVEL-set used by interrupt-disable critical sections; EXTUI is bit-field extract used by clock config; MIN/MAX were misrouted into the LSCI op0=3 group when their actual encoding is op0=0 op1=3. | Decoder + exec + Instruction enum updated. |
| 8 | **MMIO coverage gaps.** esp_hal's clock/voltage init sweeps a wide range of registers we hadn't stubbed. SYSTIMER's `UNITn_OP` had to expose VALUE_VALID (bit 29) so the snapshot-ready busy-wait exits — without this, only one Hello world! ever printed because the second `delay()` hung. | SYSTEM stub grew 0x1000 → 0x10000; RTC_CNTL grew to 0x8000 with round-trip + pre-seeded PLL_LOCK; new low/high MMIO catch-all stubs; SYSTIMER VALUE_VALID always set. |

---

## ROM Thunks Registered

All 14 thunks at addresses derived from `esp-rom-sys-0.1.4/ld/esp32s3/rom/esp32s3.rom.ld`:

| Address | Symbol | Implementation |
|---|---|---|
| 0x4000_05D0 | ets_printf | minimal `%s/%d/%i/%u/%x/%p/%c/%%` expansion → tracing::info! |
| 0x4000_0600 | ets_delay_us | NOP (sim doesn't model wall-clock) |
| 0x4000_0720 | ets_set_appcpu_boot_addr | NOP (cpu1 not modelled) |
| 0x4000_0A2C | esp_rom_spiflash_unlock | NOP, returns 0 |
| 0x4000_11F4 | memcpy | byte-wise copy via the bus |
| 0x4000_18B4 | cache_suspend_dcache | NOP, returns 0 |
| 0x4000_18C0 | cache_resume_dcache | NOP, returns 0 |
| 0x4000_1A1C | rom_config_instruction_cache_mode | NOP |
| 0x4000_1A28 | rom_config_data_cache_mode | NOP |
| 0x4000_1A4C | ets_update_cpu_frequency | NOP |
| 0x4000_2544 | __udivdi3 | 64-bit unsigned divide |
| 0x4000_057C | rtc_get_reset_reason | returns 1 (POWERON_RESET) |
| 0x4000_5D48 | esp_rom_regi2c_read | NOP, returns 0 |
| 0x4000_5D60 | esp_rom_regi2c_write | NOP, returns 0 |

---

## Plan 2 Exit Criteria Status

| # | Criterion | Status |
|---|---|---|
| 1 | Sim suite stays green | PASS — 568 tests passing |
| 2 | esp-hal hello-world builds | PASS — 2 MB ELF |
| 3 | Fast-boot synthesises correct entry state | PASS — 5 segments loaded, entry=0x40378e20, SP=0x3fcdb700 |
| 4 | E2E demo prints expected output | PASS — `e2e_hello_world` test verifies 2+ "Hello world!" lines via USB_SERIAL_JTAG sink |
| 5 | CLI runs the firmware end-to-end | PASS — `labwired run --chip … --firmware …` prints "Hello world!" |
| 6 | No silent ROM calls | PASS — every BROM PC the CPU reaches is either a registered thunk or raises NotImplemented with the address |
| 7 | Documentation | PASS — this case study |

---

## Known Gaps and Acknowledged Limitations

- **No GPIO / IO_MUX / Interrupt Matrix.** Plan 3 territory.
- **No SYSTIMER alarm IRQs.** Polling-only counter access works; alarm-driven delays land in Plan 3.
- **No HW oracle diff for the `--diff` stretch goal.** Sim and HW both produce the expected output independently; bit-stream comparison is left for Plan 2.5 if needed.
- **Static flash-XIP MMU page table.** Real firmware can remap pages at runtime; Plan 2's table is populated once at boot from segment layout.
- **`ets_delay_us` returns immediately.** Real silicon busy-waits the requested microseconds. The simulator doesn't model wall-clock time, so any boot-sequence timing dependency is absent. esp-hal hello-world doesn't depend on it for correctness; Plan 3 will need to revisit if a real timing-sensitive demo lands.

---

## Invitation for Plan 3

Plan 3 builds the next layer on top of Plan 2's boot + UART + SYSTIMER:

- **GPIO + IO_MUX:** pin functions, matrix-routed signals, edge-detect.
- **Interrupt Matrix:** 94 sources × 26 levels per core, ROM-supplied dispatch.
- **SYSTIMER alarms:** alarm registers + comparator + IRQ generation.
- **Blinky demo:** an esp-hal binary toggles a GPIO from a SYSTIMER alarm ISR; runs identically on sim and HW (with WS2812 on GPIO21 visible on the S3-Zero, OR a logic-analyzer probe on a simpler GPIO pin).

The HW-oracle infrastructure from Plan 1 extends naturally to peripheral oracle tests for GPIO + interrupt delivery.
