# Why `rec_tick=1` on each WASM family (walk-forcer inventory)

**Date:** 2026-07-27  
**Commit baseline:** branch `wasm-realtime-tick-inventory`  
**Host note:** native `cargo test` inventory (not browser). Cortex-M / C3 / RP2040 / nRF use production `SystemBus::from_config` (+ `configure_cortex_m` where applicable). **ESP32-S3** uses the production WASM path `configure_xtensa_esp32s3` (not chip-YAML `from_config` stubs).

## Why this exists

Shipped WASM was observed to recommend `peripheral_tick_interval = 1` on
h563 / rp2040 / nrf / s3, while c3 / f103 already reach **512**.

`SystemBus::max_safe_tick_interval` (see `crates/core/src/bus/policy.rs`) returns
`RECOMMENDED_TICK_INTERVAL` (512) only when **all** of:

1. `legacy_walk_disabled` (walk auto-deleted or hand-flagged)
2. `!has_iolink_master()`
3. `!hcsr04_forced_legacy` (test-only override; unused on these systems)

**H5 `flash_models_ops` is no longer a max_safe arm** (PR-D): erase/bank-swap
ops still force CPU quantum 1 via `requires_cycle_accurate` /
`Machine::apply_pending_flash_op`, but that is orthogonal to the peripheral
tick interval.

Under `event-scheduler`, `legacy_walk_disabled` auto-derives when every
peripheral satisfies `uses_scheduler() || !needs_legacy_walk()`. The walk-forcing
set is the negation:

```text
needs_legacy_walk() && !uses_scheduler()
```

This document records that set per family so PR-B–E of the monorepo WASM
real-time plan can migrate forcers (or clear non-forcer blockers) without
guessing.

Originally inventory-only; PR-B–E then migrated forcers so every shipped WASM
family now auto-derives walk-deletion and `max_safe=512` under `event-scheduler`.

## How to reproduce

```bash
cargo test -p labwired-core --features event-scheduler \
  --test tick_interval_inventory -- --nocapture
```

Source: `crates/core/tests/tick_interval_inventory.rs`  
Each case builds with `walk_deleted = None` (auto-derive).

## Summary table (event-scheduler)

| Family | System / bus path | `legacy_walk_disabled` | `flash_models_ops` | iolink | forcers | `max_safe` |
|--------|-------------------|------------------------|--------------------|--------|---------|------------|
| **stm32f103** | `examples/ssd1306-hello-lab` + `configure_cortex_m` | true | false | false | **0** | **512** |
| **esp32c3** | `configs/systems/esp32c3-devkit.yaml` | true | false | false | **0** | **512** |
| **nrf52840** | `configs/systems/nrf52840-dk.yaml` + `configure_cortex_m` | true | false | false | **0** | **512** |
| **rp2040** | `configs/systems/rp2040-pico.yaml` + `configure_cortex_m` | true | false | false | **0** | **512** |
| **stm32h563** | `configs/systems/nucleo-h563zi-demo.yaml` + `configure_cortex_m` | true | **true** (CPU q=1 only) | false | **0** | **512** |
| **esp32s3** | `configure_xtensa_esp32s3` (WASM / production) | true | false | false | **0** | **512** |

Notes:

- **C3 / F103 / nRF52840 / RP2040 / H563 / S3** are regression-green (asserted in the
  inventory test and the PR-B/C/D/E gates `nrf52840_dk_is_walk_free_and_tick_512` /
  `rp2040_pico_is_walk_free_and_tick_512` / `h563_is_walk_free_and_tick_512` /
  `esp32s3_is_walk_free_and_tick_512`).
- **S3** uses production `configure_xtensa_esp32s3` (not chip-YAML
  `from_config` stubs) and ends with `recompute_walk_deletable()` so WASM gets
  `max_safe=512` without a hand hatch. Featureless builds still report
  `max_safe=1`.
- **H563** still has `flash_models_ops=true` so CPU batches stay quantum-1
  (`requires_cycle_accurate`), but the peripheral tick interval is 512.

---

## stm32f103 — already 512

- Chip: `configs/chips/stm32f103.yaml`
- System: `examples/ssd1306-hello-lab/system.yaml` + `configure_cortex_m`
- `max_safe_tick_interval`: **512**
- `legacy_walk_disabled`: true · `flash_models_ops`: false · iolink: false
- **Forcers:** _(none)_

### Full peripheral status

| name | role | `needs_legacy_walk` | `uses_scheduler` |
|------|------|---------------------|------------------|
| rcc | inert | false | false |
| gpioa/b/c | inert | false | false |
| systick | scheduler | true | true |
| uart1/2, usart3 | scheduler | true | true |
| i2c1/2 | scheduler | false | true |
| spi1/2 | scheduler | true | true |
| afio | inert | false | false |
| exti | scheduler | false | true |
| dma1 | scheduler | false | true |
| flash_ctrl, dbgmcu, pwr, iwdg, wwdg, rtc, crc, bxcan1, usb_dev, bkp | inert | false | false |
| tim1–tim4 | scheduler | false | true |
| adc1/2 | scheduler | false | true |
| scb | scheduler | true | true |
| nvic | inert | false | false |
| dwt | scheduler | true | true |

---

## esp32c3 — already 512

- Chip: `configs/chips/esp32c3.yaml`
- System: `configs/systems/esp32c3-devkit.yaml` (plain `from_config`; no ROM inject)
- `max_safe_tick_interval`: **512**
- `legacy_walk_disabled`: true · `flash_models_ops`: false · iolink: false
- **Forcers:** _(none)_

### Full peripheral status

| name | role | `needs_legacy_walk` | `uses_scheduler` |
|------|------|---------------------|------------------|
| uart0/1 | scheduler | true | true |
| gpio | scheduler | true | true |
| timg0/1 | scheduler | true | true |
| interrupt_core0 | inert | false | false |
| system, rtc_cntl, apb_ctrl, systimer, io_mux | inert | false | false |
| i2c0 | scheduler | true | true |
| spi2 | scheduler | true | true |
| ledc | scheduler | false | true |
| rmt | scheduler | true | true |
| spi0/1, gpio_sd, efuse, uhci0/1, bb, twai0, i2s0 | inert | false | false |
| aes, sha, rsa, ds, hmac, dma | inert | false | false |
| apb_saradc | scheduler | true | true |
| usb_device, sensitive, extmem, xts_aes, assist_debug | inert | false | false |
| radio_fe, radio_nrx, wifi_mac | inert | false | false |

---

## esp32s3 — already 512 (PR-E)

- **Bus path:** `SystemBus::new()` + `configure_xtensa_esp32s3(&Esp32s3Opts::default())`
  (ends with `recompute_walk_deletable()` on the production path; inventory
  also recomputes for symmetry)
- **Not** `SystemBus::from_config` on chip YAML — that path stubs the coded S3
  models and previously produced a **false walk-free** inventory
- Mirrors: `WasmSimulator::new_from_config_xtensa_esp32s3`,
  `esp32s3_reset_conformance`, S3 e2e / walk-differential tests
- `max_safe_tick_interval`: **512**
- `legacy_walk_disabled`: **true**
- `flash_models_ops`: false · iolink: false · hcsr04: none
- **Forcers:** _(none)_

### Migration summary (38 → 0)

| Class | Models | Mechanism |
|-------|--------|-----------|
| **Class-A inert** | `intmatrix`, `core1_control`, `extmem`, `system_regs`/`system_regs_hi`, `rtc_cntl`, `gpio`, `sens_s3`, `rng`, `sha`, `hmac` | `needs_legacy_walk=false` — pure register banks / write-settled engines; `tick()` is trait-default or observer-only |
| **Class-B level-export** | `crosscore_ipi`, `pcnt`, `ledc`, `spi2/3_s3`, `i2s0/1_s3`, `twai`, `aes`, `rsa`, `sar_adc_s3`, `i2c0/1`, `gdma` | `uses_scheduler` when clock attached + `matrix_irq_sources_into` (C3 SPI pattern); GDMA transfer work stays on `tick_with_bus` |
| **Class-B real schedule** | `systimer` (factory now scheduler-mode), `timg0/1_s3`, `uart0/1/2_s3`, `rmt_s3`, `mcpwm0/1`, `ds`, `sdmmc`, `lcd_cam`, `usb_otg` | `take_scheduled_events` / `on_event` (or RMT holdoff+playback via `bus_tick` + matrix level) |

Featureless builds still report `max_safe=1` (honest). Gates:

- Inventory: `esp32s3_is_walk_free_and_tick_512` in `tick_interval_inventory.rs`
- Production configure auto-recomputes so WASM inherits tick 512 without a hatch

### Full peripheral status (production bank)

| name | role | `needs_legacy_walk` | `uses_scheduler` |
|------|------|---------------------|------------------|
| iram/dram/rtc_*/flash_*/rom/drom | inert | false | false |
| intmatrix, core1_control, extmem, system* | inert | false | false |
| rtc_cntl, gpio, sens_s3, rng, sha, hmac | inert | false | false |
| crosscore_ipi, pcnt, ledc, spi*, i2s*, twai, aes, rsa, sar_adc, i2c*, gdma | scheduler (level export) | false | true |
| systimer | scheduler | true | true |
| timg*, uart*, rmt, mcpwm*, ds, sdmmc, lcd_cam, usb_otg | scheduler | false | true |
| usb_serial_jtag, io_mux, efuse, stubs | inert | false | false |

---

## stm32h563 — already 512 (PR-D)

- Chip: `configs/chips/stm32h563.yaml`
- System: `configs/systems/nucleo-h563zi-demo.yaml` + `configure_cortex_m`
- `walk_deleted = None` (auto-derive)
- `max_safe_tick_interval`: **512**
- `legacy_walk_disabled`: **true**
- `flash_models_ops`: **true** — H5 `flash_iface` still records erase/bank-swap
  pending ops drained per instruction (`requires_cycle_accurate` → CPU quantum 1).
  **Does not pin max_safe** (policy decoupled in PR-D).
- iolink: false · hcsr04: none
- **Forcers:** _(none)_

### Migration summary (4 → 0)

| Class | Models | Mechanism |
|-------|--------|-----------|
| **Class-A inert** | `pwr` (`PwrH5`) | `needs_legacy_walk = false` — pure register bank; VOSRDY tracks VOSCR writes |
| **Class-B scheduler** | `gpdma1`, `rtc` (`RtcV3`), `fdcan1` | `uses_scheduler` + `take_scheduled_events` / `on_event` (GPDMA Dma1-style element chain; RTC second-boundary delays; FDCAN TX-defer + level IRQ). **Single-node** FDCAN is walk-free by design; **multi-node** with CanBus `bus_rx` attached forces the walk (intentional interim — not a hatch) |

Featureless builds still report `max_safe=1` (honest). Gates:

- Inventory: `h563_is_walk_free_and_tick_512` in `tick_interval_inventory.rs`
- Walk differential: `stm32h563_zephyr_boot_walk_vs_scheduler_is_byte_identical`

### Full peripheral status

| name | role | `needs_legacy_walk` | `uses_scheduler` |
|------|------|---------------------|------------------|
| rcc | inert | false | false |
| gpioa–g | inert | false | false |
| systick | scheduler | true | true |
| uart3 | scheduler | true | true |
| gpdma1 | scheduler | false | true |
| fdcan1 | scheduler (single-node); walk when CanBus attached | false (true with `bus_rx`) | true (false with `bus_rx`) |
| tim1_pwm, tim2, tim3, tim12, tim6 | scheduler | false | true |
| i2c1/2 | scheduler | false | true |
| uart1/2, lpuart1 | scheduler | true | true |
| wwdg, iwdg, rng, crc, lptim1 | inert | false | false |
| spi1/2/3 | scheduler | true | true |
| adc1 | scheduler | false | true |
| rtc | scheduler | false | true |
| nvic | inert | false | false |
| pwr | inert | false | false |
| flash_iface | inert (`flash_models_ops` → CPU q=1 only) | false | false |
| dbgmcu, icache | inert | false | false |
| scb, dwt | scheduler | true | true |

---

## rp2040 — already 512 (PR-C)

- Chip: `configs/chips/rp2040.yaml`
- System: `configs/systems/rp2040-pico.yaml` + `configure_cortex_m`
- `LABWIRED_RP2040_BOOTROM=""` (same as other RP2040 tests; bootrom is not a forcer)
- `walk_deleted = None` (auto-derive)
- `max_safe_tick_interval`: **512**
- `legacy_walk_disabled`: **true**
- `flash_models_ops`: false · iolink: false · hcsr04: none
- **Forcers:** _(none)_

### Migration summary (8 → 0)

| Class | Models | Mechanism |
|-------|--------|-----------|
| **Class-A inert** | `spi0`, `sio`, `xip_ssi` | `needs_legacy_walk = false` — pure MMIO engines; `tick()` is the default no-op (loopback/SSI shift complete inside register writes). **Walk-free Class-A is green intentional**; not a paced-SPI/I2C@512 EasyDMA certificate |
| **Class-B scheduler** | `timer`, `dma`, `pio0`, `usbctrl`, `i2c0` | `uses_scheduler` + `take_scheduled_events` / `on_event` (CycleClock on timer/dma; delay-1 SM/host/level chains on pio/usb/i2c0) |
| **Class-B / thin** | `uart0` | scheduler on the pico bus; baud / paced-transfer tick-512 fidelity is **interim** only (no EasyDMA-style differential gate) |

Featureless builds still report `max_safe=1` (honest). Gates:

- Inventory: `rp2040_pico_is_walk_free_and_tick_512` in `tick_interval_inventory.rs`
- Machine TIMER@512: `rp2040_machine_timer_alarm0_fires_at_tick_512` in
  `rp2040_timer_machine_gate.rs` — arms ALARM0 with a short target, runs through
  `Machine::advance` at `peripheral_tick_interval=512`, asserts `INTR` bit 0
  (not `tick_peripherals_fully_forced`)

### Class-B notes under `rec_tick=512`

- **TIMER**: free-running counter advances lazily via `sync_to` / read-side
  CycleClock; alarm matches fire at exact cycle deadlines; held level IRQ
  re-emits at delay 1 while `INTS != 0`.
- **DMA**: permanent-TREQ beats + level IRQ ride a delay-1 event chain (not
  `bus_tick_indices` when scheduler-owned); one beat per event cycle matches
  the walk's `BEATS_PER_TICK`.
- **PIO**: SM steps self-perpetuate at delay 1 while any SM is enabled;
  MMIO enable re-arms via `collect_scheduled_events`.
- **USB**: attach debounce / enumeration / bulk service + held `USBCTRL_IRQ`
  ride a delay-1 chain; attach countdown still counts host_poll steps (not
  wall-clock µs) — same model as pre-migration, now event-paced.
- **I2C0**: held level `(IC_RAW_INTR_STAT & IC_INTR_MASK)` rides a delay-1
  chain, armed from the MMIO write choke (the level can only RISE on a write;
  the `IC_CLR_*` registers are read-to-clear). `on_event` returns
  `raise_own_irq` — the event-path twin of the walk's `irq: true` — so
  `I2C0_IRQ` (NVIC 23) pends on an NVIC bus, where
  `deliver_scheduled_irq_levels` cannot help (it covers only the C3/S3
  matrices). Costs a wakeup only while an interrupt is armed AND asserting.
  Gate: `rp2040_i2c_irq_delivery.rs`.

### Full peripheral status

| name | role | `needs_legacy_walk` | `uses_scheduler` |
|------|------|---------------------|------------------|
| sysinfo | inert | false | false |
| dma | scheduler | false | true |
| pio0 | scheduler | false | true |
| clk_rst | inert | false | false |
| uart0 | scheduler | true | true |
| rosc, watchdog | inert | false | false |
| timer | scheduler | false | true |
| spi0 | inert | false | false |
| i2c0 | scheduler | true | true |
| systick | scheduler | true | true |
| sio | inert | false | false |
| xip_ssi | inert | false | false |
| usbctrl | scheduler | false | true |
| tbman | inert | false | false |
| scb | scheduler | true | true |
| nvic | inert | false | false |
| dwt | scheduler | true | true |

---

## nrf52840 — already 512 (PR-B)

- Chip: `configs/chips/nrf52840.yaml`
- System: `configs/systems/nrf52840-dk.yaml` + `configure_cortex_m`
- `walk_deleted = None` (auto-derive)
- `max_safe_tick_interval`: **512**
- `legacy_walk_disabled`: **true**
- `flash_models_ops`: false · iolink: false · hcsr04: none
- **Forcers:** _(none)_

### Migration summary (46 → 0)

| Class | Models | Mechanism |
|-------|--------|-----------|
| **Class-A inert** | `ficr`, `uicr`, `nvmc`, `acl`, `cryptocell`, `mwu`, `aar`, `comp`, `qdec`, `i2s`, `pdm`, `qspi`, `nfct`, `usbd`, `usbregulator`, `ppi`, `temp`, … | `needs_legacy_walk = false` (no time-driven `tick()`) |
| **Class-B scheduler** | `timer0–4`, `rtc0–2`, `wdt`, `rng`, `clock`, `egu0–5`, `gpiote`, `twim`/`serial` (i2c0, twi1), `ecb`, `radio`, **`uarte` (uart0/1), `saadc`, `pwm0–3`, `spi2` (nRF SPIM EasyDMA)** | `uses_scheduler` + `take_scheduled_events` / `on_event` (CycleClock where counters advance; EasyDMA delay-0 dual-path with `tick_with_bus`) |

Featureless builds still report `max_safe=1` (honest). Gates:

- Inventory: `nrf52840_dk_is_walk_free_and_tick_512` in `tick_interval_inventory.rs`
- Machine TIMER@512: `nrf52840_machine_timer0_compare_fires_at_tick_512` in
  `nrf52840_timer_machine_gate.rs` — programs TIMER0 with a short CC[0], runs
  through `Machine::advance` at `peripheral_tick_interval=512`, asserts
  `EVENTS_COMPARE[0]` (not `tick_peripherals_fully_forced`)
- EasyDMA@512: `nrf52_easydma_tick512_fidelity.rs` — UARTE/SAADC/PWM complete
  within ≤8 device cycles at interval 512; UARTE walk@1 vs sched@512 completion
  cycle identity within 1

### EasyDMA lag under `rec_tick=512` — **CLOSED**

**Previously (accepted interim):** UARTE / SAADC / PWM / SPIM EasyDMA completed
only via `bus_tick_indices`, so at `peripheral_tick_interval = 512` completion
could lag by up to one tick batch (~511 instructions) after STARTTX / SAMPLE /
SEQSTART / TASKS_START.

**Now:** dual-path scheduler promotions (same pattern as ECB / TWIM):

| Model | `uses_scheduler` | delay-0 token | Shared engine | `tick_with_bus` kept |
|-------|------------------|---------------|---------------|----------------------|
| UARTE | true | STARTTX → `(0, 1)` | `do_easydma_tx` | yes (bare-bus tests) |
| SAADC | true | SAMPLE → `(0, 1)` | `do_easydma_sample` | yes |
| PWM | true | SEQSTART → `(0, 1)` | `do_easydma_seq` | yes |
| SPIM (nRF) | true (already) | TASKS_START → `(0, 1)` | `do_nrf52_easydma` | yes |

Under Machine + walk-free + tick 512, completion happens on the **next cycle**
after the task write (delay-0 event), not after the 512-cycle peripheral tick
quantum. Busy-wait drivers that poll ENDTX / END / SEQEND no longer see the
batch lag.

Class-B counters (`timer*`, `rtc*`, …) already ride the scheduler; the
Machine TIMER@512 gate above is the proof that path works under batching.
See also `docs/performance/2026-07-27-fidelity-scoreboard.md`.

### RTC COUNTER read path (not yet certified)

RTC `COUNTER` advances on write/`sync_to` (scheduler), not via interior
mutability on bare MMIO read. **COUNTER-poll-only firmware is not yet
certified** at `rec_tick=512` (no dedicated poll-only differential gate);
compare-event / IRQ-driven RTC shapes are the supported surface today.

### Full peripheral status (representative)

| name | role | `needs_legacy_walk` | `uses_scheduler` |
|------|------|---------------------|------------------|
| uart0/1 | scheduler (EasyDMA delay-0 dual-path) | false | true |
| i2c0, twi1 | scheduler | false | true |
| gpio0/1 | inert | false | false |
| rtc0–2, timer0–4, wdt, rng | scheduler | false | true |
| clock, egu0–5, gpiote, radio, ecb | scheduler | false | true |
| saadc, pwm* | scheduler (EasyDMA delay-0 dual-path) | false | true |
| ppi, temp, ficr, … | inert | false | false |
| spi2 | scheduler (nRF SPIM EasyDMA delay-0 + STM32 wire) | true | true |
| scb, dwt | scheduler | true | true |
| nvic | inert | false | false |

---

## Mapping to monorepo plan (PR-B–E)

| PR | Family focus | Status / blockers |
|----|--------------|-------------------|
| **PR-B** | **nrf52840** | **DONE** — empty forcers, `max_safe=512` under `event-scheduler`; Machine TIMER@512 gate; **EasyDMA delay-0 closed** (UARTE/SAADC/PWM/SPIM) |
| **PR-C** | **rp2040** | **DONE** — empty forcers, `max_safe=512` under `event-scheduler`; Machine TIMER ALARM0@512 gate; SPI/I2C Class-A walk-free green; UART baud@512 interim |
| **PR-D** | **stm32h563** | **DONE** (single-node) — empty forcers, `max_safe=512`; `flash_models_ops` still forces CPU quantum 1 (not tick interval); **FDCAN multi-node CanBus remains intentional walk / max_safe=1 interim** until bus_rx is event-driven |
| **PR-E** | **esp32s3** | **DONE** — empty forcers, `max_safe=512` under `event-scheduler` on `configure_xtensa_esp32s3` + recompute; Class-A inert + Class-B level-export / scheduled engines |
