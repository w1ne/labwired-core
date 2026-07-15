# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.19.2] - 2026-07-15

### Fixed
- **Release runner smoke**: Bind-mounted container runs now preserve the
  caller's artifact ownership, so the published image and archive-backed
  GitHub Action can write report bundles in the same workspace.

### Changed
- **Public runner default**: The GitHub Action, integration templates, and
  documented release pins now use the verified v0.19.2 runner contract.

## [0.19.1] - 2026-07-15

### Fixed
- **Environment completion**: `inputs.env` scripts can opt into the same
  durable assertion-pass stop contract as single-machine runs, including the
  settling window and minimum-step floor. Node runtime failures and configured
  wall-time/cycle/UART limits retain precedence over completion.
- **CI runner image**: The image now builds against the same Debian/glibc
  baseline it runs on. The required Core CI check builds and executes the final
  image before a release tag can be created.

### Changed
- **Public runner default**: The one-step GitHub Action now defaults to the
  v0.19.1 immutable CLI release; release smoke exercises both that default and
  an explicit release pin.

## [0.19.0] - 2026-07-14

### Added
- **Released multi-node CI runner**: GitHub Actions and OCI release smoke tests
  can run a complete inputs.env world from YAML and publish a self-contained
  report bundle without rebuilding LabWired.
- **Environment result contract**: Multi-node runs write an explicit
  1.0-environment result schema with per-node provenance and fidelity gaps.

### Fixed
- **Environment CI safety and evidence**: Strict environment-manifest
  validation, deterministic snapshot peripheral ordering, explicit CAN
  peripheral selection, and correct safety-stop / assertion precedence make
  multi-node evidence reproducible and fail-closed.

## [0.18.0] - 2026-07-09

### Added
- **Bus-agnostic Component IR**: Declarative component models can now share one
  engine across I2C and read-only SPI devices.
- **One-step SVD ingestion**: `labwired asset ingest-svd` converts vendor SVD
  input into a runnable declarative chip path.
- **Native IO-Link simulation path**: LabWired can bridge to the native C
  IO-Link master, run cyclic PD-out frames, isolate multiple device contexts,
  and execute multi-node IO-Link worlds in CI.
- **Firmware exercise matrix**: Added an explicit matrix for which real
  firmware paths exercise each modeled board/peripheral capability, backed by
  generated tier-1 scoreboards.
- **KW41Z Zephyr and display coverage**: Added unmodified Zephyr hello,
  FXOS8700 activity, Nokia 5110 display, and visible activity-display examples.
- **Generic input and stimuli flow**: Added `SimInput` channels and declarative
  input stimuli so tests can drive modeled sensors and inputs mid-run.
- **Bus trace and analyzer exports**: Added a shared trace event stream,
  universal tracing wrappers for attached I2C/SPI devices, WASM trace draining,
  and CLI JSON/VCD export via `--bus-trace-out`.
- **Peripheral egress bridge and relay**: Simulated peripheral output can be
  forwarded through validated MQTT/TCP/HTTP-style egress flows.
- **Faithful ESP32-C3 / ESP32-S3 browser paths**: Added lazy ROM injection,
  flash-image ROM boot in WASM, ESP32-C3 SSD1306/OLED paint proof, and
  `labwired test --rom-boot` coverage for faithful ESP targets.
- **Universal peripheral inspect interface**: Added a common inspection surface
  for machine/peripheral state, including ESP32-C3 inspect integration.
- **Snapshot app-entry cache**: Faithful ESP32-C3 boot can be cached at app
  entry and resumed for faster interactive startup.
- **Additional examples and devices**: Added RP2040 Arduino Mbed-OS USB CDC,
  CAN player replay, F103 J1939 monitor, CANmod GPS simulation, display catalog
  devices, and ESP32-C3 display workshop labs.

### Changed
- **Unified pin mapping**: Chip descriptors now carry an authoritative `pins`
  map; runtime routing resolves through that map instead of parsing pin names or
  falling back silently.
- **Canonical config loading**: Rust `resolve()` now follows the TypeScript
  oracle for byte-identical canonical config behavior.
- **IO-Link wire conformance**: Device and master pins were aligned to the
  V1.1.5 spec-conformant wire mapping.
- **Browser runtime performance**: WASM builds use smaller/faster settings,
  lazy ROM loading, idle fast-forward, cached C3 tick/IRQ routing, and batched
  RISC-V stepping.
- **Test runner stop semantics**: Assertion-pass stop handling now supports a
  settle window and minimum-step floor for more stable scripted runs.
- **Release-facing README**: Rewrote the project README around concrete
  capabilities, examples, validation scope, and a verified smoke command.

### Fixed
- **IO-Link correctness and observability**: Repaired IO-Link debug/analyzer
  flow, native stack routing, multi-device state isolation, station console
  labeling, station USART scoping, and the four-port master PHY ABI.
- **nRF52 firmware fidelity**: Modeled TWIM transfer latency, both write-read
  paths, RTC LFCLK timing, and legacy UART TXD behavior for Arduino output.
- **Cortex-M and instruction fidelity**: Cleared stale pending exception bits
  after NVIC ICPR, fixed LDM T1 writeback when the base register is in the
  register list, and covered CPU arithmetic behavior used by IO-Link flows.
- **ESP32-C3 / SSD1306 paths**: Fixed OLED I2C transactions, split-command
  SSD1306 state, dynamic bus tick caching, GPIO flash boot strap values,
  read-fresh ROM-boot timers, scheduler tick walking, and workshop firmware
  display/serial behavior.
- **STM32 and RP2040 bring-up**: Modeled STM32L476 PLLSAI ready bits for
  Arduino HAL boot and RP2040 XIP_SSI for boot2/XIP completion.
- **Modeling failures become explicit**: PCD8544 D/C pin resolution and
  vendor-neutral GPIO routing now fail loudly instead of guessing a fallback.
- **CI and packaging stability**: Restored strict onboarding smoke gates, pinned
  the thumbv6m smoke linker path, reduced unnecessary GitHub Actions usage, and
  aligned the published LabWired action failure policy.

## [0.17.10] - 2026-06-29

### Fixed
- **nRF52 TWIM event register offsets**: Three event registers had wrong peripheral offsets in the model, making them invisible to firmware. The nrfx driver computes bit positions via `(offset - 0x100) / 4`, so the register at bit 18 must live at `0x100 + 18*4 = 0x148`, bit 23 at `0x15C`, bit 24 at `0x160`. The model had `EVENTS_SUSPENDED` at `0x128` (hardware: `0x148`), `EVENTS_LASTRX` at `0x158` (hardware: `0x15C`), `EVENTS_LASTTX` at `0x15C` (hardware: `0x160`). Consequences: (1) the ISR/polling loop never saw `EVENTS_SUSPENDED`, so `nrfx` never wrote `TASKS_RESUME` and the RX phase never started; (2) `EVENTS_LASTRX` was never seen at the correct offset, so completion was never acknowledged; (3) stale events at wrong offsets were never cleared by firmware, causing spurious IRQ loops that ate all simulation steps without producing UART output. Fixes Zephyr BME280 `i2c_write_read` hanging and simulation timeout.

## [0.17.9] - 2026-06-29

### Fixed
- **nRF52 TWIM TASKS_RESUME handling**: `nrfx` issues `TASKS_RESUME` (not `TASKS_STARTRX`) to restart the bus after `EVENTS_SUSPENDED` in the TX_NO_STOP (write-then-read) path. The model previously treated `TASKS_RESUME` as a no-op, causing the follow-on RX transfer to never start. The bus would hang waiting for `EVENTS_STOPPED` (from `LASTRX_STOP`), producing a simulation timeout for every I2C `write_read_dt` call. Fixed by routing `TASKS_RESUME` to `PENDING_RX` when the driver has already set up `RXD.PTR`/`RXD.MAXCNT`.

## [0.17.8] - 2026-06-29

### Fixed
- **nRF52 TWIM SHORTS register bit positions**: The `SHORTS` constants were shifted one position low relative to the nRF52840 Product Specification. `LASTTX_STOP` was at bit 8 (hardware: bit 9), `LASTRX_STOP` was at bit 9 (hardware: bit 12). The hardware uses bits 7-12: LASTTX_STARTRX(7), LASTTX_SUSPEND(8), LASTTX_STOP(9), LASTRX_STARTTX(10), LASTRX_SUSPEND(11), LASTRX_STOP(12). With the wrong positions, `nrfx_twim_xfer(XFER_RX)` wrote `LASTRX_STOP_MASK=1<<12` which was filtered out by `SHORTS_MASK`, so the STOPPED event never fired, the completion semaphore timed out (500 ms), and every I2C read returned -EAGAIN. Fixes BME280 `device_is_ready` = false in Zephyr.
- **nRF52 TWIM LASTTX_SUSPEND event**: Added `EVENTS_SUSPENDED` (offset 0x128, `INTEN` bit 18) to fire when the `SHORT_LASTTX_SUSPEND` path is taken (TX_NO_STOP mode used by `nrfx` for combined write-read transfers). Previously the model misidentified `LASTTX_SUSPEND` as `LASTTX_STOP` and fired `EVENTS_STOPPED` instead, which masked the RX semaphore timeout for the follow-on read.
- **nRF52 TWIM I2C device `stop()` on transaction end**: The TWIM model now calls `I2cDevice::stop()` when `EVENTS_STOPPED` fires (via `LASTTX_STOP`, `LASTRX_STOP`, or `TASKS_STOP`). Without this, the BME280 `register_address_written` flag was never cleared between transactions, corrupting register addressing for all transfers after the first.

## [0.17.7] - 2026-06-29

### Fixed
- **nRF52 SerialInstance pre-enable PSEL shadow**: Zephyr pinctrl writes PSEL.SCL/SDA (offsets 0x508/0x50C) before the ENABLE register is set. The SPIM0/TWIM0 mux was silently dropping those writes (falling to the no-op `_ => {}` arm), leaving PSEL at 0xFFFF_FFFF. `nrfx_twim_init` then called `nrfx_twi_twim_bus_recover(0x7FFFFFFF, 0x7FFFFFFF)` → `nrf_gpio_pin_present_check(0x7FFFFFFF)` → assertion failure at boot. Fix: route pre-enable writes/reads to the TWIM model so pinctrl's PSEL setup is preserved.

## [0.17.1] - 2026-06-20

### Fixed
- **STM32F407 SPI1 CR1 bit 12 (CRCNEXT)**: F407 silicon does not latch CRCNEXT (writing 0xFFFF reads back 0xEFFF), unlike F103 which keeps it writable. The shared classic-SPI model treated F1/F4 identically; it now applies a per-part `cr1_mask` (chip-config driven, default fully-writable 0xFFFF; F407 → 0xEFFF), mirroring the existing `cr2_mask`. Caught by the live F407 register diff.

### Added
- **hw-oracle connect-under-reset**: `LABWIRED_OPENOCD_CONNECT_UNDER_RESET` (assert SRST during connect/examine) + `LABWIRED_ADAPTER_SPEED` overrides in the OpenOCD wrapper, for boards whose running firmware disables/repurposes the SWD pins.

### Validation
- All six silicon-tier boards (stm32f103, stm32l073, stm32l476, stm32h563, stm32f407, esp32s3) re-captured on live silicon 2026-06-20; drift_acks cleared.

## [0.17.0] - 2026-06-19

### Added
- **Classical CAN (bxCAN) model** for STM32F1: loopback + frame trace, strict silicon-pinned acceptance filtering, bit-timing bus-off and bus-attach, and a real two-node CAN bus (a virtual UDS tester driving an F103 ECU).
- **RCC clock-gating model** for STM32F1 / L4: gated peripherals are inaccessible until their RCC enable bit is set (opt-in via the chip-YAML `clock:` field).
- **STM32F103C8 SRAM** modeled at its physical 20 KB.
- **nRF52840 proximity lab**: ALARM in-range threshold raised to 50 cm.

### Fixed
- **STMIA.W** decoded as STMDB (struct-copy idiom wrote below the destination).
- **Register-coverage probe** is now clock-gating-independent (`SystemBus::set_clock_gating_bypass`, measurement-only) so coverage reflects whether a register is modeled, not whether its clock is currently on. Runtime gating is unchanged.

## [0.16.0] - 2026-06-18

### Fixed
- **Bit-band translation gated on cores that have it (M3/M4)**: the bus applied Cortex-M bit-band alias translation (0x4200_0000–0x43FF_FFFF → bit ops on 0x4000_0000) to every ARM chip, but the feature exists only on Cortex-M3/M4. M33 parts (STM32H563, STM32WBA52) map their real GPIO ports at 0x4202_xxxx, so word accesses there were translated into bit-band operations and never reached the GPIO model. Chip descriptors now carry an explicit `core` field; `SystemBus::from_config` enables translation only for `cortex-m3`/`cortex-m4` (configs without a `core` field keep the historical Arm default). Un-blocks the Tier-1 `gpio` cells for `stm32h563` and `stm32wba52` and the NUCLEO-H563ZI io-smoke.
- **T1 shift-immediate flags inside IT blocks**: the 16-bit `LSL`/`LSR`/`ASR` immediate encodings updated N/Z unconditionally, but the architecture defines `setflags = !InITBlock()`. A flag update mid-IT-block re-evaluated the remaining block conditions and skipped instructions (observed as a false `gpio-bitband-shadow` FAIL in the Tier-1 H563/WBA52 fixtures after the bus fix).
- **nRF52840 build targets**: corrected pin and proximity example build targets so the nRF52840 labs build and run.
- **Proximity CLI cycle-accuracy**: the proximity demo now runs on the cycle-accurate CLI path.

### Added
- **ESP32-S3 GDMA peripheral-coupled mode**: GDMA now moves real bytes between descriptor chains and the UART (UHCI0), SPI2, SPI3, I2S0 and I2S1 models, routed by the new `IN_PERI_SEL`/`OUT_PERI_SEL` registers. UART couples through UART0's real MMIO FIFO; SPI transactions kicked with `SPI_DMA_TX_ENA`/`SPI_DMA_RX_ENA` defer completion until GDMA supplies MOSI / consumes MISO bytes (attached-device responses included); I2S streams samples gated by TX/RX_START with `RXEOF_NUM` honored as a byte count. Transfers pump incrementally (64 bytes/tick); IN (RX) descriptors are written back with the owner bit cleared and the received length in dw0[23:12], while OUT (TX) owner writeback is gated on `OUT_AUTO_WRBACK` (`OUT_CONF0` bit 2, as on silicon) — with the bit clear a completed OUT chain can be re-kicked unchanged (M2M walks follow the same writeback rules). Unmodeled peripheral ids (AES, SHA, ADC, RMT, LCD_CAM, unbound) keep the auto-complete fallback.
- **`core` field in chip descriptors** (`configs/chips/*.yaml`): exact CPU core (e.g. `cortex-m3`, `cortex-m33`, `cortex-m0+`). `arch` collapses all Cortex-M variants, so core-specific bus behavior keys off this field; SVD-IR imports derive it from the CMSIS `arch` automatically. All in-tree ARM chip yamls now declare it.
- **ESP32-S3 Faithful ROM Auto-Provisioning** (`crates/core/src/boot/esp32s3_rom.rs`): the real Espressif boot ROM is now discovered and extracted automatically from the installed toolchain (PlatformIO/ESP-IDF), cached by ELF content hash, and loaded by default — so `--rom-boot` needs only `LABWIRED_ESP32S3_FLASH` (no manual `make_esp32s3_rom_bins.py` step or `LABWIRED_ESP32S3_ROM/_DROM` env vars). The Rust extractor is byte-identical to the previous Python script. `LABWIRED_ESP32S3_ROM_ELF` overrides the ELF path; pre-extracted `LABWIRED_ESP32S3_ROM/_DROM` bins still work.
- **`Esp32s3BootMode` telemetry**: `Esp32s3Wiring.boot_mode` reports `Faithful` (real ROM) vs `Harness` (no blob found → thunk fallback); the CLI `--rom-boot` path uses it to fail clearly when no real ROM is available.
- **`LABWIRED_ESP32S3_FASTBOOT`** opt-out: forces the fast-boot/thunk path even when a real ROM is available (playground speed; deterministic fast-boot tests).
- **SoC Factory architecture**: peripherals are now built from per-family factories backed by a const peripheral table with a thin `from_config` match, replacing bespoke per-chip wiring. Generic, nRF52, and ESP32 (LX6/LX7) families migrated; adding a chip is now a table entry plus a factory hook.
- **Silicon-validation / drift gate**: a CI gate compares the model against silicon-derived expectations and fails on drift; board status (`docs`/coverage) is auto-generated from chip configs and smoke results.
- **ESP32 real-boot de-thunk**: the ESP32 (LX6) boot path runs the real ROM instead of harness thunks where a blob is available, closing the gap between faithful and fast-boot behavior.
- **Real-CAN UDS analyzer (core)**: frame-level CAN/UDS capture and decode in the core, feeding the playground logic analyzer.
- **STM32H5 FDCAN support**: M_CAN peripheral for the H5 family (fixed RAM layout), enabling H563 FDCAN labs.

### Changed
- **Module-split refactor**: the bus (routing, tick, accessors, modules), CLI command surface (run/test/snapshot/net-harness families), Xtensa core, and WASM inspect/inputs layers were split into focused modules — no behavior change, smaller compile units, clearer ownership.
- **Fast PR CI gate**: a fast core-integrity gate runs on every PR; the full suite runs post-merge (see `ci/fast-pr-gate`).
- **Data-driven coverage matrix**: the capability/coverage matrix is generated from config + smoke data rather than hand-maintained tables.
- **ESP32-S3 I2C0 interrupt source corrected** to `ETS_I2C_EXT0_INTR_SOURCE = 42` (was 49). The wrong source left the interrupt parked at a disabled CPU interrupt, so ESP-IDF's interrupt-driven `i2c_master` never completed and returned `ESP_ERR_INVALID_STATE`. The unmodified SpiceDispenser firmware now drives its PCA9685 servos over I2C on the faithful path.
- **`proper_model` (XIP flash-cache wiring) unified** with the resolved ROM: both the MMU-aware XIP model and the boot mode now derive from a single `provision_rom_images()` call, so they can never diverge.

## [0.15.0] - 2026-05-23

### Added
- **Dual-Core ESP32 / ESP32-S3 Simulation**: Round-robin step loop with PRO_CPU / APP_CPU, `PRID` register exposing `xPortGetCoreID()`, cross-core IPI bridge wiring `DPORT_CPU_INTR_FROM_CPU_n_REG` triggers into the target CPU's `INTERRUPT` bit so `esp_crosscore_int_send_yield` lands.
- **Runtime Snapshot Subsystem**: `Machine::with_secondary_cpu`, `Machine::{take,apply}_runtime_snapshot`, CLI `snapshot capture` subcommand, WASM `apply_runtime_snapshot(bytes)` + `take_runtime_snapshot()`. Cold-boot collapses from 30 s to ~0.5 s in the playground.
- **Arduino-ESP32 / FreeRTOS Bring-Up Thunks** (`crates/core/src/peripherals/esp32s3/rom_thunks.rs`): `abort_halt`, `esp_clk_cpu_freq_240mhz`, `x_queue_create_mutex_static_echo`, `x_task_get_current_task_handle`, `return_pd_true`, `spi_start_bus_fake`, `spi_class_begin_transaction` (lazy `spi_t` init with `USR_MOSI` auto-enable), `xQueueGiveMutexRecursive`, `esp_log_impl_lock`, recursive-mutex create stubs, `esp_ipc_init`/`esp_ipc_isr_init` no-ops.
- **Loader Auto-Discovery**: `extract_arduino_esp32_thunks` + `resolve_symbol_in_elf` resolve HardwareSerial / SPI / mutex / log-lock / IPC-init symbols by name from the ELF's `.symtab`, so installed thunks gate strictly on symbol presence (stripped ELFs that don't import the symbol are untouched).
- **Boot Snapshot Pipeline**: Capture a post-paint state blob from any heavy ESP32 firmware via `labwired snapshot capture` and replay it in the playground in ~0.5 s.
- **Unified Demo Registry**: `BoardConfig`-driven `fetch-demo-firmware.sh` and per-board snapshot blobs.
- **Dual-Core PxList Diagnostic Dump**: CLI flag dumps the kernel ready/delayed list state on both cores for diagnosing scheduler regressions.

### Changed
- **Xtensa `WSR.INTSET` Semantics**: SR id 226 writes now raise pending IRQ bits in `INTERRUPT` instead of being silently dropped — required for FreeRTOS `portYIELD()` software interrupt to fire (`crates/core/src/cpu/xtensa_sr.rs`).
- **CCOMPARE0 Ack-on-Write**: Writes ack bit 6 in `INTERRUPT` and re-raise if the new compare value is already ≤ `CCOUNT`, closing the silent-timer case where boot-allocator latency pushed the first compare past `CCOUNT` (`crates/core/src/cpu/xtensa_sr.rs`).
- **`xTaskGetCurrentTaskHandle`**: Removed from generic `nop_return_zero` list — now uses dedicated thunk reading `pxCurrentTCB[core]` via the `PX_CURRENT_TCB_ADDR` thread-local seed.
- **Fake `spi_t` Region**: Moved out of the firmware DRAM allocator's reach (`0x3FFD_F000` → `0x3FFF_FF00` in SRAM1) to prevent silent overwrite by heap growth.
- **`return_pd_true` Visibility**: Now emits a one-time `tracing::warn!` on first call so the stub's activation is loud in logs — future regressions where a take *should* block will be visible instead of silently succeeding.

### Fixed
- **IPC-Task Spin / Scheduler-Tick Starvation**: Combined fix from `WSR.INTSET` raising `SOFTWARE0` and `CCOMPARE0` ack-and-rearm; Arduino-ESP32 firmware now progresses past `xQueueSemaphoreTake` into `app_main` / `setup()`.
- **`esp_newlib_locks_init` Assertion Failure**: `xQueueCreateMutexStatic` returning 0 now echoes the static-buffer argument so the lock pointer is non-NULL.
- **GxEPD2 / SSD1680 SPI Writes Reach the Panel**: `SPIClass::beginTransaction` lazy init enables `USER_REG.USR_MOSI` (bit 27) when the sketch never calls `SPI.begin()` explicitly; combined with the GxEPD2 cmd/data thunks and the UC8151D panel model, an Arduino-ESP32 GxEPD2 sketch now paints the tri-color e-paper panel in sim.
- **`Print::print` / `Print::write` Dispatch**: Restored real virtual dispatch so `display.print("text")` flows `Print → Adafruit_GFX::write → drawChar → drawPixel`; only `HardwareSerial::write` and the `uart*` helpers stay stubbed.
- **Dead Inherent Watchpoint Removed**: `SystemBus::write_u32/write_u16` inherent watchpoint from #93 never fired (bus dispatch goes through trait impl) — code path removed.
- **`mkdocs.yml` Drift**: `repo_url` typo (`libwired-core` → `labwired-core`); nav entries pointing at nonexistent `SUPPORTED_DEVICES.md`, `verification_audit.md`, `development/git_flow.md` corrected or removed.
- **`SECURITY.md`**: Supported versions table updated to `0.15.x`; advisory URL corrected from `labwired` to `labwired-core`.
- **Example Lints**: Cleared `clippy::identity_op`, `needless_range_loop`, `doc_overindented_list_items`, and unused-variable warnings across sensor labs and firmware demos; gated `firmware-f407-demo` inline ARM asm behind `#[cfg(target_arch = "arm")]` so host clippy doesn't choke on it.

## [0.14.0] - 2026-05-12

### Added
- **ESP32-S3 / Xtensa LX7 Support**: Added the Xtensa LX7 CPU backend, ESP32-S3 boot path, GPIO, interrupt matrix, SYSTIMER, ROM thunk, flash XIP, USB serial/JTAG, and I2C/TMP102 support.
- **Hardware Oracle Harness**: Ported the hardware-oracle capture/replay harness to the core repo, including OpenOCD/GDB capture tooling and fixture-backed oracle tests.
- **Hardware-Validated STM32L476 Coverage**: Added the NUCLEO-L476RG board package, modern STM32L4 peripherals, CubeMX-style HAL firmware coverage, and hardware trace fixtures.
- **Hardware-Validated STM32F407 I2C Coverage**: Added STM32F407 board configs, firmware, oracle captures, survival traces, and I2C sensor coverage for AHT20/BMP280 flows.
- **Expanded Peripheral Models**: Added or extended STM32L4/L4-style peripherals including PWR, FLASH, RNG, CRC, timers, RTC, watchdogs, DAC, EXTI, DMA, SDMMC, FMC, TSC, COMP, bxCAN, SAI, USB OTG, and QSPI.
- **ISA and Snapshot Coverage**: Added ARM Thumb-2 instructions, RISC-V atomics, async IRQ fixes, snapshot schema validation, ESP32-C3 survival coverage, and an ISA coverage matrix.
- **Trace-Level Determinism Proof**: Extended `determinism.rs` to compare `trace.json` SHA-256 hashes across 5 runs; added as `determinism-proof` CI gate.
- **Deterministic Trace Serialization**: Switched `InstructionTrace.register_delta` from `HashMap` to `BTreeMap` for stable JSON key ordering.
- **Auto-Generated Compatibility Matrix**: `scripts/generate_compat_matrix.py` enumerates chip configs and smoke test coverage; output uploaded as CI artifact.
- **Conditional Breakpoints**: DAP breakpoints with `condition` and `hitCondition` expression evaluation (register comparisons, hex/decimal literals).
- **Data Breakpoints**: `supportsDataBreakpoints` DAP capability; triggers on memory writes to watched addresses via `MemoryTracker`.
- **Enhanced Evaluate Handler**: DAP `evaluate` supports `*(0xADDR)` memory dereference and `Rn +/- offset` register arithmetic.
- **Improved Disassembly**: Thumb-2 32-bit instruction decoding in DAP disassemble handler; `decode_thumb_32` re-exported from decoder module; source line correlation via DWARF symbols.

### Changed
- **Release Version**: Workspace version updated to `0.14.0` across workspace-managed crates.
- **Documentation Structure**: Consolidated architecture docs, removed stale root-level junk and orphan changelog files, corrected stale `core/...` subpath prefixes, and refreshed README positioning around hardware-validated parity.
- **Catalog Metadata**: Refreshed onboarding target pass-rate metadata and board coverage tables for modeled chips.
- **Build Profile**: Enabled thin LTO for release builds.

### Fixed
- **I2C Fidelity**: Closed STM32 I2C state-machine gaps exposed by F407 firmware and added runtime-attached AHT20/BMP280 component support.
- **Cortex-M Fidelity**: Fixed DBGMCU IDCODE behavior, vector-table handling, semihosting breakpoints, bit-band gating by architecture, and multiple Thumb-2 decode/execute gaps surfaced by hardware traces.
- **DAP Robustness**: Capped `readMemory` requests and made board I/O matching exhaustive.
- **CI and Fixture Stability**: Repaired workspace CI issues, RP2040 firmware configuration, nightly test failures, and firmware survival tests.

## [0.13.0] - 2026-03-20

### Added
- **Foundry Integration**: Core configs (`core/configs/`) now mounted into Foundry backend for dynamic catalog sync.
- **Catalog Support**: Board and peripheral YAML descriptors ingested into unified hardware catalog; validation URLs and onboarding manifests surfaced via API.

### Changed
- **Version Bump**: Workspace version updated to `0.13.0` across all crates.

## [0.12.1] - 2026-02-16

### Fixed
- **Workspace Validation**: Closed version drift between the workspace release and component manifests so release artifacts resolve to one patch line.
- **DAP Synchronization**: Aligned `labwired-dap` with the current core release and its release documentation.
- **Release Docs**: Corrected CI/test script references so release commands point at files that exist in this checkout.

## [0.12.0] - 2026-02-16

### Fixed
- **Critical Instruction Regression**: Fixed `io-smoke` failure by implementing proper **Thumb-2 `IT` (If-Then) block** support in the `CortexM` core.
- **Instruction Coverage**: Expanded modular decoder and executor for `MOVW`, `MOVT`, `LDR.W`, `STR.W`, and `UXTB.W`.
- **Structural Stability**: Refactored CPU `step` loop for improved variable scoping and exception handling consistency.

### Added
- **Documentation Overhaul**:
    - **New Site Structure**: Migrated to MkDocs with Material theme for a premium, searchable experience.
    - **Diataxis Framework**: Reorganized content into Tutorials, How-To, Reference, and Explanation.
    - **New Guides**: [`troubleshooting.md`](./docs/troubleshooting.md), [`cli_reference.md`](./docs/cli_reference.md), [`configuration_reference.md`](./docs/configuration_reference.md).
    - **Process Docs**: Added [`RELEASE_PROCESS.md`](./RELEASE_PROCESS.md) and [`board_onboarding_playbook.md`](./docs/board_onboarding_playbook.md).
- **Architecture Unification**: Native ingestion of **Strict IR** (JSON) in the simulation core.
    - Bridged `labwired-ir` with `labwired-config` via `From` traits.
    - Simulator can now load hardware models directly from SVD-derived JSON files.
- **Asset Foundry Hardening**:
    - Enhanced SVD transformation with flattened inheritance, register array unrolling, and cluster flattening.
    - Verified against STM32F4, RP2040, and nRF52.
- **Timing Hooks**: Declarative peripheral behavior for registers (SetBits, ClearBits, WriteValue) with periodic and event-based triggers.
- **Timeline View**: Professional visualization of instruction trace data in the VS Code extension.
- **Support Strategy**: Defined **Tier 1 Device Support** (STM32F4, RP2040, nRF52) in `../docs/SUPPORTED_DEVICES.md`.
- **Architecture Guide**: New comprehensive `core/docs/architecture_guide.md`.
- **SVD Ingestor**: New tool (`crates/svd-ingestor`) to generate `PeripheralDescriptor` YAMLs from SVD.
- **Strategic Horizon**: Long-term vision integrated into `../docs/plan.md`.

## [0.11.0] - 2026-02-08

### Added
- **Declarative Register Maps**:
    - **Modeling**: Enabled peripheral definition via YAML descriptors using `labwired-config`.
    - **Simulation**: Implemented `GenericPeripheral` in `labwired-core` supporting dynamic MMR modeling, bitwise masking, and reset state.
    - **Integration**: Added support for `type: "declarative"` in chip descriptors, allowing zero-code peripheral additions.
    - **Documentation**: New [Peripheral Development Guide](./docs/peripheral_development.md) for declarative IP cores.
- **ISA Extensions**:
    - **Misc Thumb-2**: Implemented `CLZ` (Count Leading Zeros), `RBIT` (Bit Reverse), `REV`, `REV16`, `REVSH` instructions.
    - **RISC-V Support**: Initial support for RV32I Base Integer Instruction Set with multi-arch GDB support.
- **Observability**:
    - **Interactive Snapshots**: Enhanced serialization for cross-architecture CPU states.

### Fixed
- **Instruction Set Coverage**:
    - **Thumb-2 Data Processing**: Fixed `thumb_expand_imm` logic for bitmask expansion (XYXY patterns).
    - **Decoder**: Resolved critical regression in **Thumb-2 `CLZ` decoding** (missing opcode range 0xFABx).
    - **Memory Access**: Standardized `F8xx` block handling for T3/T4 variants.
- **CLI Test Runner**:
    - Fixed stale snapshot type expectations in `interactive_snapshot` and `outputs` integration tests.
- **Peripherals**:
    - **UART**: Completed status register implementation with `TXE` and `TC` flags.

## [0.10.0] - 2026-02-06

### Added
- **Advanced ISA Support**:
    - **Bit Field Instructions**: Implemented `BFI`, `BFC`, `SBFX`, `UBFX` with full decoder/executor support.
    - **Misc Thumb-2 Instructions**: Added `CLZ`, `RBIT`, `REV`, `REV16` for professional firmware compatibility.
- **Peripheral Ecosystem**:
    - **ADC (Analog-to-Digital Converter)**: Modular implementation with conversion timing, interrupts, and EOC status flags.
    - **TMP102 Sensor Mock**: Concrete I2C temperature sensor peripheral for integration testing.
- **Observability & Debugging**:
    - **State Snapshots**: Full system state serialization to JSON for deterministic analysis.
    - **Modular Metrics**: Per-peripheral cycle accounting and real-time IPS reporting.
    - **GDB Remote Serial Protocol**: New `labwired-gdbstub` crate allowing connection from standard GDB clients.
    - **Interactive Debugging (DAP)**: `labwired-dap` server for VS Code integration with variable and register inspection.
- **Documentation**:
    - [Peripheral Development Guide](./docs/peripheral_development.md).
    - [Getting Started with Real Firmware](./docs/getting_started_firmware.md) onboarding guide.

## [0.9.0] - 2026-02-04

### Added
- **Testing Infrastructure**:
    - **Test Script Schema (YAML)**: Versioned schema for defining firmware tests with inputs (ELF/System), limits (steps/time), and assertions (UART contents, stop reasons).
    - **CI Regression Gates**: Enforced workspace-wide testing and linting in GitHub Actions.
    - **Pre-Release Verification**: Automated regression suite execution on release tags and PRs.
- **CI Automation**:
    - Composite GitHub Action wrapper: `.github/actions/labwired-test`.
    - CI-ready example scripts under `examples/ci/`.
- **Documentation**:
    - Updated `README.md` to reflect real-world division firmware behavior and IPS reporting.
    - Updated `plan.md` Iteration 10 with implementation details for modular observability.

### Fixed
- **CI Artifacts**: `labwired test --output-dir ...` now emits real `result.json` + `junit.xml` even on config/script errors (exit code `2`), with `status=error`, `stop_reason=config_error`, and a `message` field.

### Changed
- **CI Runner Artifacts**:
    - `result.json`: added `result_schema_version`, `limits`, and `stop_reason_details`.
    - `junit.xml`: emits one testcase per assertion to improve CI failure visibility.

## [0.8.0] - 2026-02-03

### Added
- **Observability**: Modular metrics and simulation instrumentation:
    - **SimulationObserver Trait**: Pluggable architecture for observing simulation events (reset, step, start/stop).
    - **PerformanceMetrics**: Thread-safe instruction and cycle tracking using atomic counters.
    - **Real-Time IPS**: CLI reports simulation speed (Instructions Per Second) and progress updates.
- **Modularity**: Decoupled introspection tools from the core execution engine, enabling zero-overhead simulation when observers are detached.
- **Tests**:
    - **test_metrics_collection**: Verified cycle accuracy for 16-bit and 32-bit (BL) instructions.

## [0.7.0] - 2026-02-03

### Added
- **ISA**: Advanced Thumb-2 instruction set extensions for HAL compatibility:
    - **MOV.W / MVN.W (T2/T3)**: 32-bit move/move-not with ARM-modified immediate expansion.
    - **SDIV / UDIV (T1)**: Signed and Unsigned 32-bit division instructions.
    - **thumb_expand_imm()**: Recursive immediate constant expansion for 32-bit instructions.
- **Core Peripherals**: STM32F1-compatible memory-mapped peripheral ecosystem:
    - **GPIO**: Mode config (CRL/CRH), Pin state tracking (IDR/ODR), and atomic bit manipulation (BSRR/BRR).
    - **RCC**: Reset & Clock Control enabling peripheral lifecycle management.
    - **Timers (TIM2/TIM3)**: 16-bit timers with prescaling and update interrupts.
    - **I2C**: Master mode support with status flags (SB, ADDR, TXE, etc.).
    - **SPI**: Master mode transfer simulation and status management.
- **CLI**: Advanced simulation and debugging features:
    - **Execution Tracing**: `--trace` flag for instruction-level logging with PC and opcode.
    - **Simulation Control**: `--max-steps` option to prevent infinite loops in firmware.
- **Diagnostics**: Detailed error hinting for unknown instructions (Thumb-2 vs Coprocessor vs SIMD).
- **Tests**: Comprehensive validation suite:
    - `test_mov_w_instruction` & `test_mvn_w_instruction`.
    - `test_division_instructions` for SDIV/UDIV.
    - `test_gpio_basic` for peripheral register and bit manipulation verification.
    - Total unit tests: **37**.

### Changed
- Unified 32-bit instruction reassembly logic for broader ISA support.
- Refactored `SystemBus` to pre-register core peripherals (GPIO, RCC, Timers) by default.

## [0.6.0] - 2026-02-03

### Added
- **ISA**: Real-world compatibility instruction set extensions:
    - **Block Memory Operations**: Implemented `LDM` and `STM` for efficient multi-register load/store.
    - **Halfword Access**: Added `LDRH` and `STRH` for 16-bit peripheral register access.
    - **Multiplication**: Implemented `MUL` instruction with N/Z flag updates.
- **System Peripherals**:
    - **NVIC** (Nested Vectored Interrupt Controller) at `0xE000E100`:
        - ISER/ICER registers for interrupt enable/disable
        - ISPR/ICPR registers for interrupt pending management
        - Atomic shared state architecture for thread-safe operation
    - **SCB** (System Control Block) at `0xE000ED00`:
        - VTOR (Vector Table Offset Register) support for runtime relocation
        - Shared atomic state between CPU and memory-mapped peripheral
- **Interrupt Architecture**:
    - Two-phase interrupt delivery (pend → signal) with NVIC filtering
    - External interrupts (IRQ ≥ 16) managed by NVIC ISER/ISPR
    - Core exceptions (< 16) bypass NVIC for architectural compliance
    - VTOR-based exception handler lookup in CPU
- **Bus**: Implemented `read_u16`/`write_u16` for halfword memory access
- **Tests**: Added 3 new system tests (`test_iteration_8_instructions`, `test_nvic_external_interrupt`, `test_vtor_relocation`)

### Fixed
- **Memory Map**: Corrected peripheral size allocations to prevent overlaps (SysTick: 0x10, NVIC: 0x400, SCB: 0x40)
- **CPU**: VTOR now preserved across reset for simulation flexibility

## [0.5.0] - 2026-02-03

### Added
- **ISA**: Advanced instruction support for complex C/C++ firmware initialization:
    - **Stack Manipulation**: Implemented `ADD SP, #imm` and `SUB SP, #imm` (Thumb-2 T1/T2).
    - **High Register Arithmetic**: Extended `ADD` to support high registers (R8-R15), essential for stack frame teardown.
    - **Interrupt Control**: Added `CPSIE` and `CPSID` for global interrupt enable/disable.
- **CPU**: Integrated `primask` register to track and manage global interrupt masking state.
- **Verification**: Expanded unit test suite and verified full `cortex-m-rt` boot flow compatibility.

### Fixed
- **Decoder**: Resolved opcode shadowing for `ADD` (High Register) instructions.
- **Firmware**: Updated UART1 addressing in firmware to align with STM32F103 standard descriptor.

## [0.4.0] - 2026-02-02

### Added
- **System**: Declarative hardware configuration via **System Descriptors**:
    - **Chip Descriptors**: Define SoC architecture (Flash/RAM mapping, Peripheral offsets).
    - **System Manifest**: Describe board-level wiring and external component stubs.
- **Peripherals**:
    - Full **SysTick** timer implementation (`0xE000_E010`).
    - **StubPeripheral** for functional sensor and device modeling.
- **Core**:
    - **Vector Table Boot**: Automatic loading of initial SP and PC from address `0x0`.
    - **Exception Lifecycle**: Architectural stacking and unstacking for hardware interrupts.
    - **Dynamic Bus**: Refactored `SystemBus` to support pluggable, manifest-defined components.
- **Crates**: New `labwired-config` crate for YAML-based hardware definitions.

### Changed
- CLI now supports `--system <path>` to load custom hardware configurations.
- Peripheral interaction unified under the `Peripheral` trait.

## [0.3.0] - 2026-02-02

### Added
- **ISA**: Completing critical instruction set gaps for professional firmware simulation:
    - **32-bit Support**: Implemented 32-bit instruction reassembly logic in CPU fetch loop.
    - **Advanced Data**: Added `MOVW` & `MOVT` for 32-bit immediate loading (enabling peripheral addressing).
    - **Control Flow**: Robust 24-bit Branch with Link (`BL`) reassembly and execution.
    - **Core Support**: Expanded `MOV` & `CMP` to support high registers (R8-R15).
    - **Byte Access**: Implemented `STRB` & `LDRB` for character and buffer handling.
- **Milestone**: Successfully achieved "Hello, LabWired!" simulation output via UART peripheral.

### Fixed
- **ISA**: Corrected `MOV` (High register) decoding logic.
- **Simulation**: Fixed incorrect immediate reassembly order for `MOVW/MOVT` instructions.

## [0.2.0] - 2026-02-02

### Added
- **ISA**: Expanded Instruction Set for robust firmware simulation:
    - Arithmetic: `ADD`, `SUB`, `CMP`, `MOV`, `MVN`.
    - Logic: `AND`, `ORR`, `EOR`.
    - Shifts: `LSL`, `LSR`, `ASR` (immediate).
    - Memory: `LDR` & `STR` (Immediate Offset), `LDR` (Literal), `LDR` & `STR` (SP-relative).
    - Stack & Control: `PUSH`, `POP`, `BL`, `BX`, and Conditional Branches (`Bcc`).
- **Peripherals**: UART stub implementation mapped to `0x4000_C000`.
- **Firmware**: Added `crates/firmware` demo project targeting `thumbv7m-none-eabi`.
- **Core**: Refactored `Machine` to be architecture-agnostic (Pluggable Core).

### Fixed
- **Build**: Resolved ELF load offset issue by correctly configuring workspace-level linker scripts (`link.x`).
- **ISA**: Fixed potential overflow in large immediate offsets for `LDR/STR` instructions.

### Changed
- `labwired-cli` now runs 20,000 steps by default to support firmware boot.
- Updated `docs/architecture.md` and `README.md` with new capabilities.

## [0.1.0] - 2026-02-02

### Added
- **Core**: Initial `Machine`, `Cpu`, `SystemBus` implementation.
- **Loader**: ELF binary parsing support via `goblin`.
- **Decoder**: Basic Thumb-2 decoder supporting `MOV`, `B`, and `NOP`.
- **Memory**: Linear memory model with Flash (0x0) and RAM (0x2...) mapping.
- **CLI**: `labwired-cli` runnable for loading and simulating firmware.
- **Tests**: Dockerized test infrastructure and unit test suite.
- **Docs**: Comprehensive Architecture and Implementation Plan.

### Infrastructure
- CI/CD pipelines via GitHub Actions.
- Dockerfile for portable testing.
