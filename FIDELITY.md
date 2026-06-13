# Fidelity Ledger — where LabWired cheats, so we can tighten it up

LabWired's promise is **real, deterministic, register-machinery simulation** — the
firmware drives modeled hardware registers and the modeled hardware drives the
firmware back, byte-for-byte like silicon. Anywhere we *short-circuit* that loop
— faking a function's result, mutating device state without going through the
register/bus decode, skipping a boot step, or inferring a signal we don't model
— is a **cheat**. Cheats are sometimes pragmatic (we have no boot-ROM binary to
execute), but every one is a fidelity gap we want to see and close.

## Marker convention

Every cheat in the code carries a grep-able marker on the line or block:

```
// CHEAT(<CATEGORY>): <what is faked> — real: <what silicon/real execution does>
```

Find them all:

```sh
grep -rn "CHEAT(" crates/ --include="*.rs"
```

### Categories

| Category | Meaning | How to tighten |
|----------|---------|----------------|
| `CHEAT(THUNK-ROM)` | A **boot-ROM** function is emulated in Rust because we have no ROM binary to execute (math helpers, memcpy, cache ops, ets_printf). | Map a real ESP32 ROM image and execute it. |
| `CHEAT(THUNK-LIB)` | A **firmware library** function (compiled into the ELF — FreeRTOS, heap_caps, Arduino SPI) is intercepted and faked instead of letting the real code run. The real code IS in the binary; we skip it. | Complete the peripheral models the real code needs, then drop the thunk. |
| `CHEAT(BYPASS)` | Device/peripheral state is mutated **directly**, bypassing the register/FIFO/bus decode path. | Route through the real register write → peripheral → device path. |
| `CHEAT(NOP)` | A function is replaced by a constant return (return 0 / return true / fake pointer). | Model the behavior the caller depends on. |
| `CHEAT(STUB)` | A real **peripheral** is faked as plain RAM (accepts any read/write, no behavior). | Implement the register model. |
| `CHEAT(SKIP)` | A real boot/init step is skipped and CPU state is hand-seeded (BROM skip, SP/PC seeding). | Execute the real reset/boot sequence. |
| `CHEAT(INFER)` | A heuristic stands in for a hardware signal we don't sample (e.g. command/data framing inferred from protocol state instead of the DC GPIO). | Model the real signal. |

**Not cheats (do NOT mark):** real memory backings — `iram`, `dram`,
`flash_icache`/`flash_dcache` XIP, `psram`, `rtc_slow`/`rtc_fast` — are genuine
RAM/flash regions correctly modeled as backing store. Real register models
(RCC, GPIO, SPI FIFO/CMD.USR, UART, timers, etc.) are the real machinery.

---

## Inventory (audit 2026-06-11)

### A. ESP32 ROM/library thunks — `crates/core/src/peripherals/esp32s3/rom_thunks.rs`
60 thunk fns total. Split:

- **THUNK-ROM (legit-but-gap, ~30):** `rom_memcpy/memset/memmove/memcmp`,
  `rom_ashldi3/ashrdi3/lshrdi3/divdi3/moddi3/umoddi3/udivdi3/clzsi2/ctzsi2/bswap*`,
  `rom_esp_crc8`, `cache_*` (6), `rom_config_instruction_cache_mode`,
  `esp_rom_spiflash_unlock`, `rtc_get_reset_reason`, `ets_printf`,
  `rom_cpu_freq_240mhz`/`esp_clk_cpu_freq_240mhz`/`rom_xtal_freq_40mhz`,
  `esp_rom_route_intr_matrix`, `xtos_set_intlevel`/`xtos_restore_intlevel`.
  → ROM functions; emulating is reasonable but is not executing real ROM.
- **THUNK-LIB (real cheat — skips compiled firmware):**
  `esp_idf_heap_caps_init/malloc/calloc/free/realloc` (bump allocator that
  returns REAL DRAM — memory backing, not a behaviour fake),
  `x_queue_create_mutex_static_echo` (idle-task static mutex),
  `x_task_get_current_task_handle`, `getreent_dram_fake_ptr`,
  `esp_chip_info_stub`, `xthal_window_spill_thunk`.
  - **RETIRED in the canonical proof harness** (`tests/e2e_labwired_ereader.rs`):
    `spi_start_bus_fake`, `spi_class_begin_transaction`, and the SPI bus-lock
    `xQueueSemaphoreTake/xQueueGenericSend → pdTRUE`. The real compiled
    `SPIClass::begin → spiStartBus` runs: it creates a **real** recursive bus
    mutex via `xQueueCreateMutex` (real, IRAM, backed by the heap bump pool),
    enables the SPI3 clock through real DPORT, and sets `USER.USR_MOSI`. Real
    `beginTransaction` then takes that real mutex. Whole SPI stack — bus init,
    mutex, peripheral config, data path — is real compiled firmware against real
    register models; panel still refreshes. The `spi_start_bus_fake` /
    `spi_class_begin_transaction` functions remain in `rom_thunks.rs` only
    because the cli/wasm single-core delivery wrappers still install them
    (pending boot-to-paint validation of the real path there — see priority #2).
- **BYPASS (real cheat, 0 — RETIRED):** `gxepd_write_command`, `gxepd_write_data`
  (wrote straight into the panel, bypassing SPI3) and `spi_class_transfer` (wrote
  SPI registers from Rust) are **deleted**. The real compiled GxEPD2 firmware now
  drives the panel through the real SPI3 FIFO/MOSI_DLEN/CMD.USR registers, framed
  by the firmware's own `digitalWrite(DC)` GPIO. Proven end-to-end against the
  real PlatformIO `firmware.elf`: `tests/e2e_labwired_ereader.rs` reaches a panel
  refresh via **431 real SPI3 transactions** with zero per-byte thunks; the
  register-level path is locked by `tests/e2e_spi3_dc_epaper.rs`.
- **NOP (real cheat, ~5):** `nop_return_zero` (installed at ~25 addresses),
  `return_pd_true`, `nop_return_fake_ptr`, `abort_halt`, `monotonic_counter_32`.

### B. SPI FIFO bypass — RETIRED
`push_captured_byte` (recorded bytes that skipped the FIFO/CMD.USR path for the
gxepd thunks) is **deleted**. Byte capture now happens inside
`kick_user_transaction` as the real FIFO drains, so the capture trace and the
wire are the same path.

### C. BROM skip + hand-seeded CPU — `crates/cli/src/main.rs`
Lines ~924/927, ~1729-1761, ~4061-4075: skip the boot ROM, `set_pc(entry)`,
`set_sp(0x3FFE_0000)`. SKIP. (Mirrored in `crates/wasm/src/lib.rs`.)

### C1. Dual-core handshake pre-seed + keep-alive — REMOVABLE (validated 2026-06-13)
`run_snapshot_capture` pre-seeded the SMP bring-up flags (`s_cpu_up`,
`s_cpu_inited`, `s_system_inited`, `s_resume_cores`, `s_other_cpu_startup_done`)
to 0x01 at boot, registered them with `set_appcpu_up_flags`, AND re-stamped them
every 10k cycles — so `start_other_core`/`do_other_cpu_settings` always saw
APP_CPU "up". This was a CHEAT(NOP) on the inter-core protocol.
**Finding:** gated behind `LABWIRED_NO_PRESEED=1`, the demo ESP32 e-paper ELF
(`demo-esp32-epaper-lab.elf`) runs **byte-identical** — `spi3 transactions=328`,
same end `pc=0x400d1fc2` at 20M cycles — with the pre-seed AND keep-alive fully
off. So the real APP_CPU bring-up (released via the legitimate
`ets_set_appcpu_boot_addr` ROM entry at 0x4000_689c, which unhalts the second
core at the firmware-supplied vector) already completes the handshake itself;
the pre-seed is not load-bearing. **Next:** verify across ≥2 more Arduino-ESP32
builds, then flip the default off and delete the pre-seed + keep-alive. Note the
arduino-esp32-profile comment about newer cores' tight spin-wait — confirm those
too before removing the safety net.

### D. Peripheral-as-RAM stubs — `crates/core/src/system/xtensa.rs`
Of 16 `RamPeripheral` installs, the **cheats** are the ones standing in for a
real peripheral: `slc` (SDIO host), `sdmmc_host`. The rest (`iram`, `dram`,
`flash_icache`, `flash_dcache`, `psram`, `rtc_slow`, `rtc_fast`,
`brom_low_data`, `brom_data`) are memory regions — backing store, not cheats
(though empty `brom_*` is a content gap). Also see
`crates/core/src/peripherals/stub.rs` and `esp32s3/system_stub.rs`.

### E2. DC reads low at framebuffer-write time — blank e-paper render (FIXED)
**Root cause (FIXED):** `Esp32Gpio` had no `write_u32`, so the firmware's 32-bit
`s32i` store to GPIO_OUT_W1TC (write-1-to-clear) fell back to the default
byte-split read-modify-write — and `read_word(0x0C)` returns the whole `OUT`
value, so a `digitalWrite(CS, LOW)` (`W1TC = 1<<5`) reconstructed a clear-mask
from the *current* OUT and wiped every set bit, including DC (GPIO17). GxEPD2's
`_writeData` only toggles CS and leaves DC alone, so after the first per-byte CS
toggle in the `0x24` stream, DC was gone and the framebuffer bytes were
mis-routed to `command_byte` and dropped → blank render. Fix: `Esp32Gpio` now
implements atomic `write_u32`/`write_u16` that go straight to `write_word`
(no RMW). Regression test: `gpio::tests::w1tc_via_word_store_clears_only_target_bit`.
After the fix the real firmware renders its text (black-plane 1429/4736 ink
bytes; `e2e_labwired_ereader` asserts a non-blank plane).

**Silicon cross-check (2026-06-12, ESP32-D0WDQ6 over UART, instrumented GxEPD2
tap of `(digitalRead(_dc), byte)`):** the sim is **byte-for-byte identical** to
real silicon — all **19033/19033** SPI transfers match exactly. The lone
divergence is the DC line: silicon holds **DC=1 for all 18998 data bytes** (35
commands at DC=0; the first `0x24` is followed by 300/300 transfers at DC=1),
while the sim reads DC=0 across that region. So the fix target is exact: the sim
must hold GPIO17 high across the data stream as the firmware does. Diagnostic:
cli `LABWIRED_DUMP_SPI=<path>` dumps the full wire stream; reproduce the silicon
trace by patching `GxEPD2_EPD.cpp` `_writeCommand`/`_writeData` to call an
`epd_tap(digitalRead(_dc), byte)` logger.

### E. Display command/data inference — the two e-paper panels
`uc8151d_tricolor_290.rs`, `ssd1680_tricolor_290.rs`: when no DC pin is wired,
`transfer()` guesses command-vs-data from protocol state. INFER. The live ESP32
e-paper paths (cli `arduino-esp32`, wasm, `attach_esp32_external_devices`, and
the `e2e_labwired_ereader` harness) now all wire `dc_pin` and latch the real GPIO
level, so this INFER fallback only applies when a panel is attached with no DC
source — not on the proven ereader path.

---

## North star: boot an ARBITRARY ESP32 binary
Every thunk below is located by **ELF symbol or a hardcoded PC**. A stripped /
arbitrary firmware has neither, so today only the one hand-curated `agentdeck`
build and symbol-bearing `arduino-esp32` builds boot. "Run any ESP32 binary like
real silicon" therefore is not a patch — it requires the firmware to **execute**
these paths natively, which means modeling the silicon behind them: SPI-flash
controller + flash chip, clock/RTC tree, `esp_timer` hardware, the real heap over
faithful RAM, and a mapped boot ROM. This is an SoC-model build (comparable to
the esp32c3 rom-boot effort, ×dual-core), sequenced below.

## Tightening priority (highest fidelity payoff first)

0. **Dual-core handshake** (C1) — pre-seed proven non-load-bearing; finish
   cross-firmware validation, then delete it. Cheap, near-done.

1. ~~**Drop the GxEPD2 BYPASS thunks** (A.BYPASS)~~ — **DONE.** The real compiled
   firmware drives the panel over the real SPI3 peripheral + real DC GPIO (431
   SPI3 transactions → refresh against the real ELF). Bypass thunks deleted.
2. ~~**Complete the ESP32 SPI library path**~~ — **DONE in the e2e proof harness:**
   real `spiStartBus` + real `xQueueCreateMutex` + real `beginTransaction` run
   against the DPORT/heap/SPI models; the SPI fakes are removed there. **Remaining:**
   apply the same removal to the cli (`arduino-esp32`) and wasm delivery wrappers,
   which still stub `xQueueCreateMutex`→NULL + `spiStartBus`/`beginTransaction` +
   the bus-lock `pdTRUE` for their single-core snapshot boot. Needs cli/wasm
   boot-to-paint validation (the cli binary can boot the ELF; wasm runs in-browser).
3. **Model `slc`/`sdmmc_host`** instead of RAM stubs, or prove the firmware never
   needs them.
4. **Real boot ROM** — execute a mapped ROM image to kill the THUNK-ROM set and
   the BROM SKIP at once. PROGRESS (2026-06-13):
   - **FIXED — boot-index trap.** Real BROM (`tests/fixtures/esp32_brom.elf`) used
     to run 12,948 instrs then spin at PC=0x40007bcc ("ets_main.c:404"). RE'd it:
     `main` calls `rtc_get_reset_reason()` (0x400081d4), which returns
     `RESET_STATE[5:0]` — that value (17) was the out-of-range boot index. Root
     cause: `rtc_cntl.rs` packed the reset-cause fields as 4-bit (APP_CPU at bit 4),
     but the BROM decodes 6-bit fields (PRO=`[5:0]`, APP=`[11:6]`; verified by
     `extui 0,6` / `extui 6,6`). With both causes=POWERON(1) the model produced
     0x11=17. Fix: `RESET_CAUSE_APPCPU_SHIFT 4→6`, `MASK 0xF→0x3F`. BROM now runs
     **1,000,000+ instrs, no fatal stall** (was 12,948). All 18 rtc_cntl tests pass;
     e-paper still paints (756 ink). Silicon-accurate, benefits any reset-reason reader.
   - **NEXT stall:** the BROM proceeds toward loading the app from SPI flash
     (`ets_unpack_flash_code`). To boot-to-app the smoke test needs a flash image +
     the SPI-flash controller / MMU model. Then TIMG/RTC clock. Phased,
     board-validatable.
5. **heap_caps / FreeRTOS** — let the real allocator + scheduler run once the
   memory map + timers are faithful.
