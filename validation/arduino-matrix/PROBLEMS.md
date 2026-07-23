# Arduino matrix — problems found (2026-07-22)

**Hard rule: NO THUNKS.** Gaps are fixed by modeling real silicon (MMIO, memory
map, clocks, dual-core, FPU, etc.). Do not add Arduino/ESP “quirk” harnesses,
flash-patched stubs, or forge SMP handshakes as the product path.

Full matrix: 15 chips × 3 sketches. Scoreboard:
[`docs/coverage/arduino-scoreboard.md`](../../docs/coverage/arduino-scoreboard.md)

## Green (Arduino L0–L2 on plain `labwired test`)

| Chip | Notes |
|------|--------|
| nRF52840 | Already worked |
| STM32F103 | Already worked |
| STM32F401 | Already worked |
| STM32L073 | Already worked |
| STM32L476 | Already worked |
| **STM32F407** | **Fixed** — added real PWR + FLASH peripherals |
| **ESP32 classic** | **Fixed L0–L2** — dual-core + NVS + AHB UART; plain `labwired test` → `LW_L0_OK` / `LW_L1_OK` / `LW_L2_OK` |
| **STM32H563** | **Fixed L0–L2** — VFP `VCVT` / `VMOV.F32 #imm` for Arduino M33 startup |
| **STM32G474** | **Fixed L0–L2** — V2 RCC `PLLCFGR`/`ICSCR` storage so `GetSysClockFreq` + clock config complete |
| **STM32WB55** | **Fixed L0–L2** — system memory + HPREF/EXTCFGR + `USART1_IRQn=36` (was wrongly 37) |
| **nRF52832** | **Fixed L0–L2** — RTC1 @ `0x40011000` (was mis-mapped RADIO); real UART model |
| **RP2040** | **Fixed L0–L2** — bootrom `@0` must win over low-address flash alias; minimal B0-compatible `rom_func_lookup` |
| **ESP32-C3** | **Fixed L0–L2** — FreeRTOS yield + factory MMU/`cache2phys` + C3 RMT for RGB `LED_BUILTIN` (pin 30 → `rgbLedWrite`) |
| **ESP32-S3** | **Fixed L0–L2** on plain `labwired test` — dual-core + ROM strlen + RMT TX-done IRQ holdoff for RGB `LED_BUILTIN` |
| **STM32WBA52** | **Fixed L0–L2** — in-tree PIO board (Arduino Core NUCLEO_WBA55CG), stm32duino `STM32WBAxx` series fix, PWR `SVMSR` ACTVOS/ACTVOSRDY |

## Open gaps (model work only)

| Chip | Symptom | Honest next model work |
|------|---------|-------------------------|
| _(none)_ | All 15 chips × L0–L2 green on plain `labwired test` (2026-07-23) | — |

~~**STM32WBA52**~~ was a PIO board gap; closed via in-tree `pio-boards/nucleo_wba52cg.json` + stm32duino WBA series patch + PWR `SVMSR` ACTVOSRDY.

## Fixes already landed (honest)

1. `configs/chips/stm32f407.yaml` — `pwr` @ 0x40007000, `flash` @ 0x40023C00  
2. `configs/chips/nrf52832.yaml` — FICR, UICR, RADIO, `memory_regions` for Nordic errata probe @ 0xF0000FE0  
3. **ESP32 classic SPI0/1 flash commands** — `Esp32Spi` answers JEDEC RDID (`0x9F` → W0/`RD_STATUS` = Winbond `0x001640EF`), RDSR/WREN, auto-clears dedicated FLASH_* bits. `init_flash` / `esp_flash_init_main` return ESP_OK.  
4. **ESP32 dual-core + DPORT CACHE_STATE** + post-BROM `g_rom_flashchip` seed (DRAM state, not a firmware patch).  
5. **CLI/diag install of `xthal_window_spill_nw` CPU-model spill workaround** (shadow-window vs firmware spill → `0xfffffff0`; same as e2e_ereader — not a flash-init thunk).  
6. **Xtensa window hybrid preserve (real FW past `esp_intr_alloc`)** — CALL{n} keeps caller a0..a{n*4-1} on a dedicated `call_preserve_stack` (not mixed into per-slot LIFO). RETW does classic LIFO restore only for displaced callee-window slots (WS re-set), then restores preserve from `call_preserve_stack`. Fixes steal of outer CALL8 a5 after 16-slot wrap (`esp_intr_alloc_intrstatus_bind` / `heap_caps_malloc`). Unit tests: `window_tests::{call8,nested,deep_call8_wrap}_*`. Diag: a5 stays 0 across malloc; `s_system_full_inited=1`, scheduler running.  
7. **Spill thunk** — flattens `call_preserve_stack` into physical ARs, clears preserve + shadow, sets `WINDOWSTART = 1<<WB` (ROM-accurate) so outer frames reload via WindowUnderflow after context-switch.  
8. **IRQ defer after windowed ROM thunk return** — `return_with` sets `defer_irq_until_retw` until the caller's RETW. Prevents timer ISR between `_xtos_set_intlevel` (unmask) and `vPortExitCritical`'s RETW from desyncing WindowBase (was: a6=&xKernelLock → 1, mux=1 spin).  
9. **IRQ call_preserve depth guard** — on interrupt entry snapshot preserve depth; on RFE truncate so ISR RETWs cannot steal the interrupted task's outer CALL snapshots.  
10. **Spill ABI + IRQ-entry spill (SMP shared SP)** — `xthal_window_spill` writes OF/UF layout (`a0..a3 @ callee_sp-16`, `a4..a7 @ parent_sp-32`) without flattening preserve into physical ARs (WB wrap clobbered live SP). Sets `WINDOWSTART=1<<WB`. Same spill on interrupt entry so FreeRTOS interrupt-path switches have stack save areas; RFE restores preserve only on same-SP (else clear — was grafting idle preserve onto ipc1 → shared `pxTopOfStack` / IDLE1 suspended / SMP assert).  
11. **Per-TCB preserve stash + relative RETW restore (flash IPC)** — park `call_preserve` under `pxCurrentTCBs[core]` on IRQ entry; on task-switch RFE restore it and re-apply outer panes. `restore_call_preserve` writes panes **relative to current WB** (absolute slots after switch put a5/a6 in a9/a10). Unblocked `ipc_task` a5/a6 so `spi_flash_op_block_func` runs; `s_flash_op_complete=1`.
12. **Hybrid IRQ spill must not write BSS** — `spill_call_preserve_to_stack` used `mem[frame_a1-12]` as parent_sp for CALL8 a4..a7. Unprimed OF save areas hold BSS pointers (e.g. `0x3ffc2248`); writing a4..a7 to `parent_sp-32` overwrote `s_no_block_func[1]` with `0x3ffc2228` during Level5/`esp_ipc_isr` mid `flash_op_block`. Second `esp_ipc_call_nonblocking` CAS then always failed. Fix: only spill through stack-range SPs (≤4 KiB of live SP); self-parent when link is not stackish. Result: flash_op_block ×2, clear stays 0, boot reaches `initArduino`.
13. **Classic ESP32 flash image + MMU + ROM MD5 (OTA path)** — Hybrid XIP windows (`ClassicFlashWindow`): dirty overlay pages serve ELF load; clean pages with valid PRO MMU entry serve shared flash backing. MMU tables at `0x3FF10000`/`0x3FF12000`. Real `esp_rom_md5_{init,update,final}` (classic `MD5Context`). Seed `partitions.bin` @ flash `0x8000` + app XIP MMU for `cache2phys` → `esp_ota_get_running_partition` returns app0 (no OTA firmware thunk). Diag: nvs_flash_init hit; flash_block ×4.
14. **ROM `esp_rom_crc32_le` @ 0x4005cfec** — IEEE CRC-32 LE (core dump init after partitions).
15. **SPI NOR page-program / sector-erase** — FLASH_PP/SE/BE + USR 0x02/0x20/0xD8 update flash backing (NVS format path).
16. **Spill: no self-parent a4–a11 OF write** — `frame_a1-32` self-parent stomps current ENTRY locals; only spill a4–a11 for a true parent SP (strictly above this frame).

17. **Spill: 16-byte-aligned SP only; no a4–a11 stack write on IRQ** — hybrid preserve panes with `a1=load_sp+36` (data pointer) wrote OF a0–a3 through free-list at sp+24. Reject non-16B-aligned bases; park a4–a11 only in per-TCB preserve (RFE restore).
18. **Classic DPORT peripheral IRQ routing** — `aggregate_esp32_classic_irqs` maps `explicit_irqs` sources through PRO/APP MAP tables into `pending_cpu_irqs[core]` (UART0 source 34 → APP slot when loopTask is on core 1).
19. **UART AHB FIFO aliases** — `UART_FIFO_AHB_REG` @ `0x60000000`/`0x60010000`/`0x6002E000` share FIFO/sink with APB UARTs (`uart_ll_write_txfifo`).
20. **ESP32-C3 app-entry SP + ROM auto-load** — RISC-V `setup_and_run` seeds SP at top of chip DRAM (Arduino `call_start_cpu0` assumes bootloader left a stack; was SP=0 → fault `0xfffffffc`). `from_config` loads in-tree `crates/core/roms/esp32c3/{esp32c3_rom,esp32c3_drom}.bin` when `LABWIRED_ESP32C3_ROM[_DATA]` unset.
21. **Cortex-M VFP `VCVT` + `VMOV.F32 #imm`** — Arduino STM32H563 M33 startup uses `vcvt.f32.s32` / `vcvt.u32.f32` / fixed-point `#fbits` / `vmov.f32 #1.0`. Decode + execute; L0–L2 green on plain `labwired test`.
22. **STM32WB55 system memory** — `memory_regions` @ `0x1FFF0000` (32 KiB) so option-byte / package fingerprint reads do not bus-fault.
23. **V2 RCC (G4/WB) completeness** — store `ICSCR`@0x04 + `PLLCFGR`@0x0C (needed by `HAL_RCC_GetSysClockFreq`); on CFGR write set WB prescaler-applied flags **HPREF/PPRE1F/PPRE2F** (bits 16–18); model **EXTCFGR**@0x108 with **SHDHPREF/C2HPREF**. G474 L0–L2 green; WB55 past `SystemClock_Config` into Arduino `setup`/`loop`.
24. **STM32WB55 USART1 IRQ** — chip yaml had `usart1.irq: 37` (that is **LPUART1_IRQn**); correct **USART1_IRQn = 36** so TXEIE IRQ-driven `HardwareSerial` TX reaches the sink. L0–L2 green.
25. **nRF52832 RTC1 + UART** — RADIO was wrongly base-mapped to `0x40011000` (that is **RTC1**, PS §6.21); Arduino `millis()`/`delay()` poll RTC1 COUNTER@0x504 so delay hung. RADIO moved to real `0x40001000`; added RTC1 (`num_cc: 4`) + NVMC; switched `uart0` from generic `profile: nrf52` (wrong TXDRDY offset) to **`nrf52840_uart`** (legacy TXD@0x51C → TXDRDY@0x11C). L0–L2 green.
24. **ESP32-C3 fast-boot peripherals (CLI)** — BROM `.data` unpack; SPIMEM0/1; ANA_I2C; cache; SYSTIMER; SARADC; MMU table + DROM FlashXip (identity seed); partitions.bin @ flash `0x8000`.
25. **ESP32-C3 boot SP placement** — SP must be `< SOC_DRAM_HIGH (0x3FCE0000)` for cache-freeze stack sanity **and** below BROM `.data` (~`0x3FCDE710..0x3FCE0000`). Use `0x3FCDC000`.
26. **RP2040 bootrom vs flash low alias** — optimized `read_u16`/`read_u32` checked the Cortex-M boot alias (`addr → flash.base+addr`) before `extra_mem`, so mask ROM at `0x0` was shadowed by XIP (e.g. PC `0xa10` ran `CallbackBase::destroy` from `0x10000a10` → fault `0xd0071e0c`). Order is now `extra_mem` then flash alias. In-tree minimal B0-compatible bootrom (`crates/core/roms/rp2040/bootrom.bin`) via `memory_regions` + `LABWIRED_RP2040_BOOTROM`. L0–L2 green (USB CDC Serial).
27. **ESP32-C3 FreeRTOS first yield** — disable CLINT `mtimecmp=u64::MAX` (line 7 is matrix, not MTIP); `esp32c3_irq_routing` + SYSTEM `CPU_INTR_FROM_CPU_0` @ `0x600C_0028` → source 50 → `esp_crosscore_isr` → `vPortYieldFromISR`. Without this, `vTaskStartScheduler` returns into `start_cpu0` infinite loop.
28. **ESP32-C3 factory MMU / cache2phys** — C3 MMU entry is `(vaddr>>16)&0x7F` (IROM `0x4200_xxxx` and DROM `0x3C00_xxxx` share the table). Map factory app at flash `0x10000` so `cache2phys(code)` lands in app0; identity-to-0 made `esp_ota_get_running_partition` abort. Leave free MMU entries for `spi_flash_mmap` of partitions @ `0x8000`.
29. **ESP32-C3 RMT (L2 RGB LED)** — Arduino `LED_BUILTIN`/pin 30 is `RGB_BUILTIN` → `rgbLedWrite` → RMT TX. Minimal C3 RMT @ `0x60016000` (CONF0 @ `0x10`, INT_RAW @ `0x38`, source 28): instant `TX_END` on `TX_START`. L0–L2 green on plain `labwired test`.

## Explicitly rejected

- `esp32_arduino` CLI module / `install_arduino_esp32_bootstrap`  
- Wiring wasm `install_arduino_esp32_quirks` into `labwired test` as the solution  
- Forging `s_cpu_up` / heap_caps_* flash patches for matrix green  

## Re-run

```bash
cd core
python3 validation/arduino-matrix/run_matrix.py
python3 validation/arduino-matrix/run_matrix.py --boards stm32f407,nrf52832
```

30. **ESP32-S3 Cache_Suspend_ICache** — ROM suspend/enable/disable drive EXTMEM `CACHE_STATE` (was nop → IRAM poll hang).
31. **ESP32-S3 dual-core APP_CPU** — `Machine::with_secondary_cpu` so `s_system_inited[1]` is set by real `do_system_init_fn` (not flags-only).
32. **Spill `dram_sp` includes S3 DRAM** `0x3FC8_0000..0x3FCF_0000` (classic-only range left `xthal_window_spill` empty → WindowUnderflow a1=0).
33. **ESP32-S3 `intr_matrix_set` ROM** @ `0x40001b54` — harness nop left FROM_CPU/systimer unmapped; yield never fired (only first task).
34. **ESP32-S3 factory MMU + MMU-XIP fast-boot** — `seed_factory_mmu_for_cache2phys` + `factory_flash_base=0x10000` for cache2phys; partition mmap still empty → OTA `it != NULL` assert.
35. **ESP32-S3 ROM MD5** @ `0x40001c5c/68/74` — partition table MD5 verify (was harness nop → empty table → OTA assert).
36. **ESP32-S3 SYSTIMER legacy tick** — factory uses `new_with_source_legacy_tick` (scheduler-driven never advances without `event-scheduler` feature).
37. **`px_current_tcb` multi-chip** — hybrid CALL preserve parks under FreeRTOS TCB via `pxCurrentTCBs[core]`. Classic-only `0x3FFC_27C8` left S3 at `0x3FC9_B2B4` unparked → APP `_WindowUnderflow8` loop after `vTaskDelay`. Now: ELF `PX_CURRENT_TCB_ADDR` + probe classic/S3 DRAM TCB pointers.
38. **ESP32-S3 ROM `strlen` @ `0x40001248`** — harness prefill is `nop_return_zero`; `Print::write(const char*)` got length 0 so `Serial.println("LW_L0_OK")` wrote nothing. Real `rom_strlen` thunk; memcpy/memset already registered.
39. **ESP32-S3 DRAM spill/TCB range** — hybrid spill + `looks_like_tcb` use `0x3FC8_8000..0x3FD0_0000` (was capped at `0x3FCF_0000` / 480 KiB model).
40. **ESP32-S3 RMT TX-done IRQ holdoff** — instantaneous TX_END+IRQ on `TX_START` re-entered `rmt_tx_default_isr` while IDF `rmt_transmit` still held locks → FreeRTOS `xTimerQueue` corruption (`EnterCritical` @ `0x20406a`). Latch INT_RAW immediately; suppress level IRQ for ~2000 sim ticks so WaitBits can arm. Arduino L2 `LED_BUILTIN`/`rgbLedWrite` green.

41. **RP2040 bootrom auto-load** — `from_config` loads in-tree `roms/rp2040/bootrom.bin` when `LABWIRED_RP2040_BOOTROM` is unset (empty env opts out for bare-metal flash-alias PIO onboarding). Plain `labwired test` no longer needs the matrix to export the env.

42. **RP2040 L2 step budget** — mbed RTX `delay()` is SysTick-driven (~`SystemCoreClock/1000` steps/ms). L2 setup `delay(10)` + loop `delay(20)×2` before `LW_L2_OK` exceeds 5M; board `max_steps: 20_000_000` (same class as F407/G474 L2).

43. **ESP partition table beside firmware** — matrix wiped `_pio_work/<cell>` after each run while CLI only fell back to hard-coded L0 `partitions.bin`, so L1/L2 saw empty flash@0x8000 → `No MD5 found` / exception. Runner copies PIO `partitions.bin` next to `firmware.elf`; CLI `resolve_esp_partitions_bin` prefers that path (classic/C3/S3).

44. **STM32G474 L2 step budget** — `max_steps: 20_000_000` so setup `LW_L2_BOOT` + blink delays reach `LW_L2_OK`.

45. **STM32WBA52 Arduino path** — in-tree `pio-boards/nucleo_wba52cg.json` (Arduino Core `NUCLEO_WBA55CG` / NOD_WBA52CG); matrix `boards_dir` + runner auto-applies `scripts/patch_stm32duino_wba_series.py` so PIO does not mis-detect series as `STM32WBxx`.

46. **STM32WBA PWR SVMSR** — Arduino `HAL_PWREx_ControlVoltageScaling` polls `SVMSR`@0x3C `ACTVOSRDY`/`ACTVOS` after programming `VOSR`. Synthesize ACTVOS from VOSR.VOS and assert ACTVOSRDY with VOSRDY so clock config leaves the timeout spin.
