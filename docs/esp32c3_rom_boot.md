# ESP32-C3 `--rom-boot`: running unmodified IDF binaries

This documents the plumbing that lets LabWired boot an **unmodified ESP32-C3
ESP-IDF firmware image** from the real mask ROM, exactly as silicon does — no
recompiled firmware, no faked entry point. It is the "run the binary, doesn't
matter who compiled it" path.

Enable it with the CLI flag `--rom-boot` on a RISC-V chip config:

```
LABWIRED_ESP32C3_ROM=/path/esp32c3_rom.bin \
LABWIRED_ESP32C3_ROM_DATA=/path/esp32c3_rom_data.bin \
LABWIRED_ESP32C3_FLASH=/path/flash.bin \
labwired run --chip configs/chips/esp32c3.yaml --firmware app.elf --rom-boot
```

The wiring lives in `crates/cli/src/main.rs` (`run_firmware_riscv`, the
`--rom-boot` branch). The copyrighted mask ROM is **never committed** — it is
loaded at runtime from the `LABWIRED_ESP32C3_ROM` / `_ROM_DATA` paths (dump it
from live silicon with `openocd dump_image`).

## Boot chain reached

```
reset @0x40000000 → mask ROM → 2nd-stage bootloader (SHA-verifies + loads app)
  → call_start_cpu0 → app_init → heap_init → spi_flash → FreeRTOS scheduler
  → main_task → app_main() → esp_wifi_init → esp_wifi_start → phy_init → RF cal
```

## Memory regions (`configs/chips/esp32c3.yaml`)

| Region     | Base         | Size  | Purpose |
|------------|--------------|-------|---------|
| `iram`     | 0x4037_C000  | 384K  | instruction RAM (app IRAM segment) |
| `drom`     | 0x3C00_0000  | 8M    | flash DROM XIP window (rodata) |
| `rtc_fast` | 0x5000_0000  | 8K    | RTC FAST retention RAM (`esp_rtc_get_time_us`) |
| `rom`      | 0x4000_0000  | 384K  | mask ROM (`LABWIRED_ESP32C3_ROM`, not committed) |
| `rom_data` | 0x3FF0_0000  | 128K  | ROM constant data (`LABWIRED_ESP32C3_ROM_DATA`) |

## Peripheral models wired for rom-boot

All registered in the `--rom-boot` branch, overriding the declarative stubs.
Bus routing uses **greatest-start-wins** among windows containing an address,
so a narrower, later-registered window overrides a broad declarative one.

| Name | Addr | Model | Why it's needed |
|------|------|-------|-----------------|
| `spimem1_flash` / `spimem0_flash` | 0x6000_2000 / 0x6000_3000 | `esp32s3::spi_mem_flash` | BROM READ/RDID/RDSR return real flash bytes |
| `mmu_table` | 0x600C_5000 | `Esp32s3MmuTable` | flash-cache virtual→physical page table |
| `flash_irom_xip` / `flash_drom_xip` | 0x4200_0000 / 0x3C00_0000 | `FlashXipPeripheral` (MMU_FMT_C3) | app runs from flash via the MMU, like silicon |
| `extmem_cache` | 0x600C_4000 | `esp32c3::cache` | auto-completes ICache invalidate/sync done bits |
| `sha` | 0x6003_B000 | `esp32c3::sha` | real SHA-256 so the bootloader accepts the app image |
| `wdev_rnd` | 0x6002_60B0 | `esp32c3::rng` | fresh RNG word/read (`bootloader_fill_random`) |
| `rtc_cntl_timer` | 0x6000_8000 | `esp32c3::rtc_timer` | advancing RTC slow-clock timer — see below |
| `systimer` | 0x6002_3000 | `esp32s3::systimer` (`new_with_source(_,37)`) | 16 MHz counter + FreeRTOS tick alarm |
| `apb_saradc` | 0x6004_0000 | `esp32c3::sar_adc` | ADC self-cal conversion-done bit |
| `rtc_i2c_ana` | 0x6000_E000 (0x400) | `esp32c3::ana_i2c` | analog-I2C master FSM/cal done bits — see RF cut |
| `radio_fe_pll_lock` | 0x6000_6174 | `esp32c3::forced_status` | RF PLL-lock bit — see RF cut |

Plus power-on reset state seeded directly into RTC_CNTL / GPIO_STRAP / eFuse:
- `0x6000_8038 = 1` — RTC reset cause = POWERON_RESET (BROM bails on 0).
- `0x6000_4038 = 8` — GPIO_STRAP = SPI fast-flash boot.
- `0x6000_8850 bits[20:18] = 4` — eFuse wafer version → chip rev v0.4
  (the 2nd-stage bootloader rejects the app below v0.3).

## Why the RTC timer is load-bearing

The single highest-leverage model. `rtc_time_get` latches the RTC slow-clock
counter (write `RTC_CNTL_TIME_UPDATE` @0x6000_800C bit31, read TIME0/TIME1 @0x10
/0x14). A **frozen** counter wedges every RTC-deadline wait — most visibly
`calibrate_ocode` (RTC bandgap offset-code cal), which polls a regi2c comparator
that never settles without real RF and relies on a ~10 ms RTC timeout to give
up. With a real advancing timer the loop hits that timeout, prints
`W rtc_init: o_code calibration fail`, and continues — which is exactly
silicon's graceful no-RF path, not a thunk. The counter advances one tick per
simulated step (`tick()` at the default `peripheral_tick_interval = 1`); only
monotonic advancement matters, not the absolute slow-clock rate.

## RISC-V interrupt delivery (FreeRTOS scheduler)

FreeRTOS's first context switch is a `vPortYield` that writes the SYSTEM
FROM_CPU IPI register and expects the CPU to trap into the scheduler. The C3
uses a custom interrupt controller, not standard PLIC/CLINT:

- **CPU** (`crates/core/src/cpu/riscv.rs`): takes external interrupt lines
  **1..31** (`mcause = 0x8000_0000 | line`); the vectored `mtvec`
  (`_vector_table` @0x4038_0000) dispatches to `_interrupt_handler`, which reads
  `mcause & 0x1F` for the line. MPIE/MPP are saved on trap and restored on MRET.
  `WFI` is a no-op busy-wait (the idle task spins on it; interrupts are polled
  every step). `mtimecmp = u64::MAX` disables the internal CLINT timer so a
  self-pending MTIP (bit 7) can't collide with ESP matrix line 7.
- **Bus** (`crates/core/src/bus/mod.rs`, `aggregate_esp32c3_irqs`): each tick,
  routes asserted sources — the SYSTEM FROM_CPU IPI regs (0x6000_0028..0x34 →
  sources 50..53) and peripheral `explicit_irqs` (the SYSTIMER tick alarm =
  source 37) — through the INTERRUPT_CORE0 matrix MAP regs (0x600C_2000 +
  source*4) into CPU lines, gated by `CPU_INT_ENABLE` (0x600C_2104) and per-line
  priority (0x600C_2114 + line*4) vs `CPU_INT_THRESH` (0x600C_2194). The result
  is a level-sensitive line bitmask the core ORs into `mip` via
  `Bus::external_irq_lines()`; a line drops the tick after its source
  de-asserts (e.g. the ISR clearing FROM_CPU), so no latched re-trap. Gated by
  `bus.esp32c3_irq_routing` (set only on this path); zero effect elsewhere.

The C3 SYSTIMER reuses the ESP32-S3 `Systimer` IP via `new_with_source(160 MHz,
37)` — same registers, but the C3 maps TARGET0/1/2 to matrix sources 37/38/39
(the S3 uses 57/58/59).

## The RF air-gap cut

A pure simulator has no physical radio. The closed `libphy` RF calibration
(`txdc_cal`, `pll_cal`, …) launches analog/RF/PLL operations and busy-polls
hardware status bits — PLL-lock, calibration "done", FSM-idle — that only real
RF would assert. We run real register/instruction models everywhere up to that
boundary and **report the RF side as ready/locked/done**:

- `esp32c3::ana_i2c` (0x6000_E000): the analog-I2C master. Reads of the status
  word (0x50 bits[26:24]=7, FSM idle/done) and the cal command/status word (0x4C
  bit24, transaction done) always report complete, so the ROM clock/PLL
  bring-up and `txdc_cal` busy-polls exit. Other regs are register-backed for
  the `rom_i2c_*` read-modify-write path.
- `esp32c3::forced_status` (generic): register-backed window that ORs a
  configured set of `(offset, mask)` status bits into reads. Used at
  `radio_fe` 0x6000_6174 bit16 (RF PLL lock) so `pll_cal` completes. New RF
  lock/done bits discovered during bring-up are added as `(offset, mask)`
  entries here.

This is consistent with the project's "real models, not thunks" rule: the cut
is at the irreducible RF air-gap, not in the MAC/protocol logic.

## Debugging

- `LABWIRED_TRAP_DEBUG=1` — `handle_trap` prints the first ~60 traps
  (`cause`/`epc`/`mtvec`) — invaluable for the interrupt path.
- `--break-at 0xADDR` — enables a recent-PC trail, per-20M-step progress
  prints, bad-jump trapping, and surfaces halts (otherwise silent).
- Resolve a PC to its IDF function via the app ELF symbols (`riscv32-esp-elf-nm`
  / `objdump`); ROM PCs disassemble from the `LABWIRED_ESP32C3_ROM` dump
  (`objdump -D -b binary -m riscv:rv32 --adjust-vma=0x40000000`).
