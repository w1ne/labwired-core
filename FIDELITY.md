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
- **THUNK-LIB (real cheat — skips compiled firmware, ~12):**
  `esp_idf_heap_caps_init/malloc/calloc/free/realloc`,
  `x_queue_create_mutex_static_echo`, `x_task_get_current_task_handle`,
  `spi_class_begin_transaction`, `spi_start_bus_fake`, `getreent_dram_fake_ptr`,
  `esp_chip_info_stub`, `xthal_window_spill_thunk`.
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

### D. Peripheral-as-RAM stubs — `crates/core/src/system/xtensa.rs`
Of 16 `RamPeripheral` installs, the **cheats** are the ones standing in for a
real peripheral: `slc` (SDIO host), `sdmmc_host`. The rest (`iram`, `dram`,
`flash_icache`, `flash_dcache`, `psram`, `rtc_slow`, `rtc_fast`,
`brom_low_data`, `brom_data`) are memory regions — backing store, not cheats
(though empty `brom_*` is a content gap). Also see
`crates/core/src/peripherals/stub.rs` and `esp32s3/system_stub.rs`.

### E. Display command/data inference — the two e-paper panels
`uc8151d_tricolor_290.rs`, `ssd1680_tricolor_290.rs`: when no DC pin is wired,
`transfer()` guesses command-vs-data from protocol state. INFER. The live ESP32
e-paper paths (cli `arduino-esp32`, wasm, `attach_esp32_external_devices`, and
the `e2e_labwired_ereader` harness) now all wire `dc_pin` and latch the real GPIO
level, so this INFER fallback only applies when a panel is attached with no DC
source — not on the proven ereader path.

---

## Tightening priority (highest fidelity payoff first)

1. ~~**Drop the GxEPD2 BYPASS thunks** (A.BYPASS)~~ — **DONE.** The real compiled
   firmware drives the panel over the real SPI3 peripheral + real DC GPIO (431
   SPI3 transactions → refresh against the real ELF). Bypass thunks deleted.
2. **Complete the ESP32 SPI library path** so the remaining `spi_class_*` /
   `spi_start_bus_fake` THUNK-LIB init shims die. These no longer touch the data
   path (registers + DC are real); what's left is one-time bus init: faking the
   `spi_t` (so `_spi->dev`/`USER.USR_MOSI` are set) and short-circuiting the SPI
   bus-lock mutex (NULL lock in the faked `spi_t`). Retiring them means running
   the real `spiStartBus` (DPORT clock-enable + GPIO matrix + a real FreeRTOS
   mutex) instead of the fake.
3. **Model `slc`/`sdmmc_host`** instead of RAM stubs, or prove the firmware never
   needs them.
4. **Real boot ROM** — execute a mapped ROM image to kill the THUNK-ROM set and
   the BROM SKIP at once.
5. **heap_caps / FreeRTOS** — let the real allocator + scheduler run once the
   memory map + timers are faithful.
