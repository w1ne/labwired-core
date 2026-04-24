# ESP32-S3-Zero Digital Twin — Design Document

- **Date:** 2026-04-24
- **Status:** Approved for implementation planning (pre-plan)
- **Target release:** `labwired-core v0.12.0`
- **Author / owner:** Andrii Shylenko
- **Reference hardware:** Waveshare ESP32-S3-Zero board (ESP32-S3FH4R2 SoC, 4 MiB in-package flash, 2 MiB in-package PSRAM, native USB-C → USB-Serial-JTAG, WS2812 on GPIO21)

## 1. Goal

Onboard the **Waveshare ESP32-S3-Zero board** as a first-class target of `labwired-core`. The deliverable is a digital twin of the board: every unit of simulated CPU and peripheral behavior is cross-validated against the physical S3-Zero over JTAG via a hardware-oracle test harness that runs in CI. The simulator must be able to execute bare-metal Rust firmware built with `esp-hal` and produce a trace that matches the real board's observable behavior, bit-for-bit where deterministic and within documented tolerances elsewhere.

## 2. Non-goals (locked)

These are explicitly out of scope for MVP. They are neither secretly in scope nor "nice-to-have-if-time-permits":

- WiFi and Bluetooth / BLE — `WIFI_*` and `BT_*` MMIO regions fault on access.
- Crypto engines (AES, SHA, RSA, HMAC, DS / XTS_AES) — stubbed to return zero on reads, accept writes, no side effects (firmware reads them at boot; faulting would break boot).
- RMT, I²S, TWAI, LCD_CAM, SDMMC, USB OTG host/device — regions fault on access.
- Cache-line hit/miss state at line granularity — modeled as aggregate counters only.
- DMA cycle-accurate timing — functional semantics only (descriptors walked, bytes moved).
- ESP-IDF + FreeRTOS boot path (`--boot=full`) — deferred to v1.1; MVP uses fast-path boot (`--boot=fast`).
- GDB stub for Xtensa — architecture preserved to plug in; implementation deferred.
- ADC (APB_SARADC) — deferred.
- Pipeline-accurate CCOUNT (branch prediction, forwarding stalls) — approximate cost table in MVP; deferred.

## 3. Context

`labwired-core` today supports ARM Cortex-M (M0/M3/M4) and RISC-V RV32I, with a declarative YAML-driven peripheral engine, SVD ingestor, GDB/DAP debug adapters, and a CI test runner. STM32F103 is the only fully onboarded chip; `configs/systems/*.yaml` "boards" today have empty `external_devices:` lists.

This project adds:

- A new CPU backend for the **Xtensa LX7** core family.
- A full ESP32-S3 chip definition + on-chip peripheral set.
- A complete **ESP32-S3-Zero board** (chip + external devices as wired by Waveshare, plus a `physical_hardware:` descriptor that pairs the board YAML with a connected USB device).
- A hardware-oracle harness that drives the attached S3-Zero over OpenOCD JTAG.
- A hybrid trace / diff toolchain (`hw-trace`, `hw-runner`, `labwired compare`).
- A set of bare-metal Rust fixture firmwares (vendored from `esp-hal` + custom) with committed golden traces.

## 4. Architecture

### 4.1 Three-layer split

1. **`crates/core/` — extended, not forked.**
   - New `cpu/xtensa_lx7.rs`, `cpu/xtensa_regs.rs`, `cpu/xtensa_sr.rs`, `cpu/xtensa_fp.rs`.
   - New `decoder/xtensa.rs` + `decoder/xtensa_narrow.rs`.
   - New `peripherals/esp32s3/` submodule with hand-coded peripherals where SVD is insufficient.
   - Bus gets word-granular trigger events for declarative peripherals (fixes the existing TODO in `peripherals/declarative.rs`).

2. **`configs/` — pure data.**
   - `configs/svd/esp32s3.patched.svd` — vendored from `esp-pacs` with attribution.
   - `configs/chips/esp32s3-fh4r2.yaml` — chip definition for the exact SoC variant in the S3-Zero (4 MiB flash, 2 MiB PSRAM).
   - `configs/systems/esp32s3-zero.yaml` — the board: chip reference + external devices + physical-hardware descriptor.
   - `configs/peripherals/ws2812.yaml` — reusable external device (addressable RGB LED).
   - `configs/peripherals/esp32s3/*.yaml` — auto-ingested peripheral descriptors (UART, SYSTIMER, TIMG, LEDC, I²C, SPI, etc.).

3. **New workspace crates.**
   - `crates/hw-trace/` — shared trace event model + VCD + Perfetto JSON writers; diff engine. Used by both sim and HW-runner.
   - `crates/hw-runner/` — host-side binary: flashes firmware via `espflash`, drives OpenOCD, captures RTT + USB-CDC + optional sigrok, emits the shared trace format.
   - `crates/hw-oracle/` — OpenOCD TCL bridge + `#[hw_oracle_test]` proc-macro for sim↔HW cross-validation tests.
   - `crates/esp32s3-fixture/` — Xtensa target crate: CI regression binaries + shared runtime (`labwired_trace!`, `LWTRACE_SYNC` emitter, structured panic handler).

4. **Existing `crates/cli/` gets new subcommands:** `compare`, `capture`, `diff`, and an extended `run --trace`.

5. **Vendored third-party:**
   - `vendor/esp-hal-<tag>/` — pinned-tag copy of `esp-hal` (MIT/Apache-2.0) with attribution. Fixture firmwares link to this local path, not crates.io, so upstream version drift cannot break CI without an explicit refresh PR.

### 4.2 Board onboarding convention

`labwired-core` distinguishes between **onboarding a chip** and **onboarding a board**. The S3-Zero work delivers both:

- **Chip YAML** (`esp32s3-fh4r2.yaml`): memory map, on-chip peripherals, interrupt routing, CPU count, FPU presence.
- **Board YAML** (`esp32s3-zero.yaml`): references the chip, then:
  - `external_devices:` — WS2812 on GPIO21, BOOT button on GPIO0, etc.
  - `pin_map:` — silkscreen labels for each exposed GPIO (so examples match the physical board).
  - `physical_hardware:` — new YAML key: USB VID/PID, chip revision, OpenOCD config target. `hw-runner` uses this to recognize a connected device and refuse mismatched firmware.
- **Golden-reference pairing:** the board YAML's `golden_traces:` list points at committed VCDs for each fixture firmware, captured from a calibrated physical board.

## 5. CPU and decoder structure

### 5.1 Module layout

```
crates/core/src/
  cpu/xtensa_lx7.rs       ~2.5k LoC — state, fetch loop, exec dispatch, exception/interrupt, dual-core scheduler
  cpu/xtensa_regs.rs      ~400 LoC — windowed register file (64 AREGs), WindowBase/WindowStart, rotation helpers
  cpu/xtensa_sr.rs        ~500 LoC — SR table
  cpu/xtensa_fp.rs        ~800 LoC — f0..f15, BR b0..b15, FP insn exec, CPENABLE/CP-exception gating
  decoder/xtensa.rs       ~1.8k LoC — length predecoder + main decode
  decoder/xtensa_narrow.rs ~300 LoC — 16-bit Code Density forms
```

### 5.2 Fetch loop (must be right before anything else works)

1. Read 32 bits at PC, unaligned-safe.
2. Length predecode on byte 0: narrow vs wide. Misaligned PC (odd byte) must fetch, not trap — only the decoder determines length.
3. Dispatch to `decode_wide(u32)` or `decode_narrow(u16)` → typed `Instruction` enum.
4. Execute; PC advance 2 or 3 accordingly.
5. Trace hook emits `(cpu_id, pc_before, insn_bytes, regs_touched, bus_ops)` to the trace sink.

### 5.3 Per-core register state

- **Physical AR register file:** 64 × 32-bit, indexed as `physical[(WindowBase*4 + ar_idx) mod 64]`.
- **WindowStart:** 16-bit bitmap, one bit per 4-register chunk, tracks spill state.
- **PS:** fielded struct with ring, EXCM, WOE, INTLEVEL, CALLINC, OWB.
- **SRs (MVP set):** EPC1..6, EPS2..6, EXCSAVE1..6, EXCCAUSE, EXCVADDR, DEPC, INTERRUPT, INTENABLE, INTCLEAR, SAR, LBEG/LEND/LCOUNT, VECBASE, CPENABLE, THREADPTR, CCOUNT, CCOMPARE0/1/2, SCOMPARE1, LITBASE, M0..M3/ACCLO/ACCHI (stubs for context save/restore), FCR/FSR (FP).
- **FP state:** f0..f15, BR b0..b15.
- **ICache/DCache MMU state** lives on the bus (EXTMEM peripheral), not the CPU.

### 5.4 Exception/interrupt dispatch

- Vector address = `VECBASE + fixed_offset_per_vector_type`. VECBASE is runtime-mutable (esp-hal relocates vectors from ROM to image at boot).
- On exception: `EPC[INTLEVEL] = PC`, `EPS[INTLEVEL] = PS`, `PS.EXCM = 1`, `PS.INTLEVEL = level`.
- Window overflow/underflow have dedicated 64-byte vector slots at `VECBASE + 0x0..0x180`.
- `S32E`/`L32E` decode to real exec only when `PS.EXCM = 1` (vector-context-only opcodes).
- `RFE`/`RFI`/`RFWO`/`RFWU`/`RFDE`/`RFDO` restore PS and PC from the appropriate shadow level.

### 5.5 ISA option cut list (LX7 on ESP32-S3)

| Option | Decision | Rationale |
|---|---|---|
| Base core ISA (~80 ops) | Required | Nothing works without it |
| Windowed Registers (CALL4/8/12, ENTRY, RETW, ROTW, S32E, L32E) | Required | esp-hal `Reset` uses `entry` immediately; rustc esp-rs toolchain defaults to windowed ABI |
| Code Density (~26 narrow ops) | Required | Ubiquitous in compiler output |
| Zero-Overhead Loop (LOOP, LOOPNEZ, LOOPGTZ, LBEG/LEND/LCOUNT) | Decoded + stubbed | Not emitted by rustc in normal code; SRs must latch for context save/restore coherency |
| MUL32 / MUL16 / DIV32 / MUL32_HIGH | Required | Emitted by rustc |
| MAC16 (M/ACC SR reads+writes) | Stubbed | Not emitted by rustc; SR save/restore must not fault |
| Bit-manip (NSA, MIN/MAX, SEXT, CLAMPS, ADDX2/4/8, SUBX2/4/8) | Required | Emitted by rustc intrinsics |
| L32R | Required (PC-relative only; LITBASE writes no-op not fault) | Every function uses it |
| S32C1I + SCOMPARE1, L32AI, S32RI | Required | Rust atomics on Xtensa |
| Boolean Registers (b0..b15) | Required | Coupled to FP compares |
| FP Coprocessor (single-precision, ~60 ops + FCR/FSR) | **Required** | User decision: full digital twin |
| CPENABLE + CP-disabled exception | Required | esp-hal uses lazy FP enable path |
| FLIX / Wide branches / Predicted branches / Exclusive / MMU | Skip | Not present on S3 |
| Debug (BREAK, DEBUGCAUSE, DDR, partial ICOUNT/IBREAKA/DBREAKA) | Partial | Required for panic traps; full set is v1.1 |
| THREADPTR | Required | TLS; xtensa-lx-rt saves it |

Approximate MVP instruction count: **~215 distinct encodings** (155 integer + windowed + density + MUL + bit-manip + ~60 FP). Estimated decoder+exec LoC: 5–6k Rust.

### 5.6 Dual-core scheduler

- Two LX7 cores (PRO + APP). Both cores **implemented and tested** in MVP. Same startup semantics as real silicon: at reset, PRO executes from the ELF entry; APP is held in reset (`SYSTEM_CORE_1_RUNSTALL=1`, `SYSTEM_CORE_1_RESETING=1`, `SYSTEM_CORE_1_CLKGATE_EN=0`) and is released only when firmware writes the APP-release sequence to `SYSTEM.CORE_1_CONTROL_0`. Firmware that never releases APP simply sees a halted second core — identical to real hardware.
- Round-robin quantum, default **16 instructions per core** (not 1024). Small quantum is deliberate — priority is fidelity, not speed.
- Event-driven reschedule points override the quantum: atomic ops (`S32C1I`, `L32AI`, `S32RI`), memory barriers (`MEMW`, `EXTW`), peripheral side-effects that raise interrupts, cross-core register writes. Firmware cannot observe a scheduling granularity above what the real bus arbiter shows.
- Both cores share one `SystemBus`. `S32C1I` holds a bus lock for the RMW duration, taking priority over the other core's pending access.
- Per-core trace streams are tagged `cpu0` and `cpu1` throughout VCD and Perfetto output.
- Inter-core signalling (`CPU_INTR_FROM_CPU_0..3`) routes through the interrupt matrix like any peripheral IRQ.

### 5.7 Floating-point strategy

- Register file: `f0..f15`, FCR, FSR. CPENABLE gates instruction execution; first FP use with CPENABLE=0 raises Coprocessor Disabled exception (EXCCAUSE=32).
- Arithmetic: `softfloat` crate (MIT/Apache), bit-exact IEEE 754 with configurable rounding mode and denormal-flush, matching LX7 FPU behavior. **Not** host `f32` — fidelity priority.
- FP compares use BR (Boolean Register) writes; coupling implemented faithfully.
- xtensa-lx-rt's lazy-FP handler path (set CPENABLE in CP-disabled exception, return) is exercised unchanged.

### 5.8 Cycle counting

- Per-instruction cost table: 1 cycle for reg-reg ops, 2 for loads (cache hit), 20–40 for loads (flash-XIP miss), 3 for taken branches, etc.
- Cache-hit and bus-contention effects modeled approximately at the bus layer.
- Full pipeline accuracy (branch prediction, forwarding stalls) is **v1.1**. MVP "1:1" means observably equivalent, not gate-level.
- CCOUNT tolerance in oracle tests: exact for reg-reg; ±2 cycles for memory ops in MVP, tightened in v1.1.

### 5.9 Global time base

- Simulator maintains one `picoseconds` monotonic clock.
- CPU cycles convert via configured frequency (default 240 MHz for S3).
- Peripherals (SYSTIMER, TIMG, UART baud, WS2812 bit-time) tick against the picosecond clock, not against instructions. Sim-vs-HW trace timestamps are therefore directly comparable.

## 6. Memory map, bus, and boot path

### 6.1 Memory regions (ESP32-S3-Zero, ESP32-S3FH4R2 variant)

| Region | Base | Size | Notes |
|---|---|---|---|
| Internal SRAM, IRAM alias | `0x40370000` | 448 KiB | Unified 512 KiB SRAM, I-bus view |
| Internal SRAM, DRAM alias | `0x3FC88000` | 480 KiB | Same physical SRAM, D-bus view |
| Internal BROM (mask ROM) | `0x40000000` | ~384 KiB | Reset handler + ROM API |
| Internal DROM | `0x3FF00000` | ~64 KiB | ROM constants |
| Flash-XIP I-cache window | `0x42000000` | up to 32 MiB | MMU-paged; backed by 4 MiB in-package flash |
| Flash-XIP D-cache window | `0x3C000000` | up to 32 MiB | DROM / PSRAM data; backed by 2 MiB in-package PSRAM |
| RTC FAST RAM (CPU view) | `0x600FE000` | 8 KiB | Deep-sleep survivable |
| RTC SLOW RAM | `0x50000000` | 8 KiB | Aliased at `0x600FE000` on S3 |
| Peripheral MMIO | `0x60000000` | up to `0x600D1000` | See §7 |

### 6.2 Bus architecture

- Single `SystemBus` shared by both cores.
- Request carries `(cpu_id, addr, width, access_type, cycle_ts)`.
- Address routing is a 4 KiB granularity region table, built at chip-YAML load time.
- Dual-aliased IRAM/DRAM windows map to the same backing store; writes through either alias are coherent.
- Atomic ops hold a bus lock across RMW, giving observable `compare_exchange` semantics to SMP firmware.
- Declarative peripherals get word-granular trigger events (fixes the existing byte-granular TODO in `declarative.rs`). This is pre-requisite work: must land before any peripheral that triggers on 32-bit MMIO writes can function.

### 6.3 EXTMEM cache/MMU model

- Not a pipeline cache — an address-translation layer for flash/PSRAM windows.
- Page table: 64 entries × 64 KiB = 4 MiB per direction.
- Firmware programs via `EXTMEM_PRO_ICACHE_MMU_TABLE_*` (esp-hal's `configure_cpu_caches()` path writes these).
- Fast-path boot pre-populates the table from the ELF segment layout, so `main` is reachable immediately.
- Cache maintenance ops (`Cache_Suspend_DCache`, `Resume`, etc.) are NOPs returning success codes.

### 6.4 Boot paths

- `--boot=fast` (MVP default): ELF loader places segments directly, synthesizes post-bootloader CPU/memory state (PS=0x10, WindowBase=0, WindowStart=1, VECBASE=0x40000000, stack at `_stack_start_cpu0`, EXTMEM pre-populated), PC = ELF entry. No BROM involvement.
- `--boot=full` (v1.1): BROM image loaded; reset PC = `0x40000400`; BROM stubs implemented as Rust functions intercepted at known ROM addresses. Second-stage bootloader loaded from emulated SPI flash at offset 0x0.

### 6.5 ROM stub policy

Fast-path boot still needs ROM stubs (firmware calls them explicitly). BROM address range is populated with redirect-to-Rust-fn thunks. Stubs needed for MVP: `ets_printf`, `ets_set_appcpu_boot_addr`, `rom_config_instruction_cache_mode`, `Cache_Suspend_DCache`, `Cache_Resume_DCache`, `esp_rom_spiflash_*`. Same Rust functions are reused when `--boot=full` lands.

### 6.6 Backing storage

`hw-runner`/`cli` takes `--flash firmware.bin`. The file is memory-mapped read-only through EXTMEM translation. The board YAML names in-package sizes (4 MiB flash, 2 MiB PSRAM for the S3-Zero).

## 7. Hardware Oracle Methodology (digital-twin guarantee)

This section is load-bearing: it's the mechanism by which we guarantee the sim is a digital twin and not a lookalike.

### 7.1 Principle

No sim component is considered complete until it has been cross-validated against the physical S3-Zero over JTAG. Every instruction, every peripheral register, every exception vector ships with an **oracle test** that runs the same stimulus on sim and on real silicon and diffs the observable state bit-for-bit.

### 7.2 JTAG access path

- **OpenOCD with Espressif's ESP32-S3 LX7 Xtensa config** (`target/esp32s3.cfg`). The mature path for Xtensa on ESP32-S3. probe-rs Xtensa support is experimental and reserved for later.
- Built-in USB-Serial-JTAG on the S3-Zero (`303a:1001`) works directly with OpenOCD; no external probe.
- `crates/hw-oracle/` wraps OpenOCD's TCL interface in a Rust control library: `halt()`, `resume()`, `step()`, `read_ar(idx)`, `read_sr(name)`, `read_mem(addr, len)`, `write_mem(addr, bytes)`, `read_peripheral_reg(name, offset)`, `capture_trace_until(breakpoint)`.
- Single-shot per test: flash tiny asm test program → halt at `BREAK` → pull state → diff.

### 7.3 Oracle test taxonomy (in CI from day one)

1. **ISA oracle tests** — ~215 tests, one per MVP encoding family. Hand-written Xtensa asm: set input regs, execute target insn, `BREAK 1,15`. Dump AR + relevant SRs, diff.
2. **Window-machine oracle tests** — CALL4/8/12 + ENTRY + RETW chains forcing WindowStart bit transitions + overflow/underflow exceptions. Dump WindowBase/WindowStart + physical register file.
3. **Exception/interrupt oracle tests** — every EXCCAUSE, every interrupt level, check EPC/EPS/EXCSAVE/EXCCAUSE/EXCVADDR shadow stacks, VECBASE relocation, PS.EXCM/WOE/INTLEVEL transitions.
4. **Peripheral register oracle tests** — per register, per access: bit-exact read-back, per-write side effects, interrupt assertion, FIFO flow.
5. **Peripheral trace oracle tests** — full-firmware: run a fixture on HW with RTT capture (always) + GPIO waveform capture via sigrok-cli (optional, only when a logic analyzer is attached to the HW-oracle rig), run same on sim, diff VCD line-by-line. Tests that require GPIO capture are marked `#[hw_oracle_test(requires = "sigrok")]` and skipped when the analyzer is absent.
6. **Dual-core oracle tests** — SMP ping-pong via `CPU_INTR_FROM_CPU_n`, spinlock with `S32C1I`, shared counter increments.
7. **FP oracle tests** — every FP insn with edge-case inputs (NaN payloads, subnormals, infinities, rounding-boundary values). Capture f0..f15 + FSR exception flags.
8. **Timing oracle tests** — CCOUNT values at instrumented points. Exact for reg-reg; ±2 cycles for memory ops in MVP.

### 7.4 Oracle test runner

- `crates/hw-oracle/`: depends on `openocd` (subprocess) and `espflash` (library).
- `#[hw_oracle_test]` proc-macro, analogous to `#[test]`, generates both sim run and HW run, diffs automatically, fails on mismatch.
- OpenOCD sessions serialized via a mutex file so concurrent `cargo test` cannot stomp each other.
- Gated behind `--features hw-oracle`. Local `cargo test` runs sim-only; `cargo test --features hw-oracle` runs against the S3-Zero. CI runs with HW gating on a self-hosted runner.

### 7.5 Development workflow enforcement

- Writing a new instruction: PR template requires oracle test + passing diff output attached.
- Writing a new peripheral: register-level oracle + at least one sequence trace oracle.
- Changing existing behavior: all oracle tests re-run before merge.
- A component without an oracle test is **not merged**. This is the mechanical guarantee of digital-twin fidelity.

### 7.6 Reference source priority

1. First authority: the real S3-Zero, via oracle test.
2. ESP32-S3 TRM + Cadence Xtensa ISA Reference Manual, for understanding *why* the hardware behaves as it does.
3. Espressif QEMU source, `xtensa-lx`, `xtensa-lx-rt` — **read only**, never for code copy (QEMU is GPL-2.0).
4. Renode Xtensa notes, Espressif `xtensa-isa-doc` — MIT-clean references for peripheral-register semantics.

## 8. Peripheral module plan

Every row names source of register defs, hand-coded bits (what SVD can't express), oracle test family, IRQ sources, and rough schedule slot.

Legend: **SVD** = auto-ingested. **HAND** = hand-written Rust. **HYBRID** = both.

| # | Peripheral | Source | Hand-coded bits | Oracle family | IRQ sources | Week |
|---|---|---|---|---|---|---|
| 1 | Interrupt Matrix | HAND | 94 src × 26 ext mux per core; priority; per-core mask | per-source routing | — (routes all) | 4–5 |
| 2 | GPIO + IO_MUX | HAND | Cross-peripheral signal routing; pin func-select; pull-up/pull-down; drive strength | pin-toggle trace, interrupt-on-edge | GPIO | 8 |
| 3 | UART0/1/2 | HYBRID | FIFO depth + flow, baud-rate timing, break/parity detect, CDC auto-wiring for UART0 | byte-stream diff, baud-timing diff | UART0/1/2 | 7 |
| 4 | USB_SERIAL_JTAG | HYBRID | CDC endpoint bridge to host; JTAG endpoint stubbed | byte-stream diff | USB_SERIAL_JTAG | 7 |
| 5 | SYSTIMER | HYBRID | 2×64-bit counters, 3 alarms each, load/update handshake, cross-core read coherency | long-running alarm, CCOUNT cross-check | SYSTIMER_TARGET0/1/2 | 7 |
| 6 | TIMG0/1 | HYBRID | WDT `0x50D83AA1` unlock sequence; 64-bit counter; alarm compare | WDT feed-or-die | TG0/1_T0_LEVEL, TG0/1_WDT_LEVEL | 8 |
| 7 | LEDC | HYBRID | Phase counter, hpoint/lpoint compare, fade IRQ, 8 ch × 4 timers | PWM waveform VCD diff | LEDC | 10 |
| 8 | I²C0 | HYBRID | Command FIFO, START/STOP/RSTART, SCL/SDA open-drain waveform, clock stretch, ACK/NACK | bus-waveform diff, tmp102 round-trip | I2C_EXT0 | 10 |
| 9 | SPI2 (GP-SPI) | HYBRID | Transaction sequencer, MOSI/MISO/SCK/CS waveform, 4 CS, 1/2/4/8-bit modes; PIO only | transfer VCD diff | SPI2 | 11 |
| 10 | SYSTEM | HYBRID | Clock gating (observable via CCOUNT); `CORE_1_CONTROL_0` triggers APP-core release | dual-core bring-up | — | 4 (min) / 11 (full) |
| 11 | RTC_CNTL | HYBRID | sw_cpu_stall bits; rest cosmetic stubs | register readback | — | 4 (stubs) |
| 12 | EXTMEM | HAND | Cache MMU page table; `Cache_*` ROM thunks | `main` reachable through flash-XIP | — | 10 |
| 13 | GDMA | HAND (stub) | Descriptor ring walker; memcpy semantics; no cycle timing in MVP | byte-delivery check | GDMA_IN/OUT_CHn (fire-on-done) | 11 |
| 14 | EFUSE | HAND (stub) | Fixed MAC, chip-rev, flash-size for fields esp-hal reads at boot | readback | — | 4 |
| 15 | APB_SARADC | SKIP | — | — | — | v1.1 |

### 8.1 External devices on the S3-Zero board

| Device | YAML | Simulated as | Oracle |
|---|---|---|---|
| WS2812 RGB LED on GPIO21 | `configs/peripherals/ws2812.yaml` + hand-coded timing decoder | One-wire bit-time parser attached to GPIO21 observer; decodes to RGB; writes color trace | Real LED waveform via logic analyzer; diff decoded colors |
| BOOT button on GPIO0 | declarative | Pullup-to-low stimulator | Manual stimulus during reset |

### 8.2 Non-peripheral stubs for boot

`APB_CTRL`, `UHCI0/1`, `HMAC`, `DS`, `AES`, `SHA`, `RSA`, `XTS_AES` — stubs return zero on read, accept writes, no side effects (firmware reads them at boot).

`WIFI_*`, `BT_*`, `RMT`, `I2S0/1`, `TWAI`, `LCD_CAM`, `SDMMC` — unmapped regions raising bus faults (surfaces accidental usage early — chosen deliberately for strictness; silent stubs would hide bugs).

### 8.3 Module layout

```
crates/core/src/peripherals/esp32s3/
  mod.rs
  intmatrix.rs       (hand)
  gpio_matrix.rs     (hand)
  iomux.rs           (hand)
  uart.rs            (hybrid)
  usb_serial_jtag.rs
  systimer.rs
  timg.rs
  ledc.rs
  i2c.rs
  spi.rs
  system.rs
  rtc_cntl.rs
  extmem.rs
  gdma.rs
  efuse.rs
external_devices/
  ws2812.rs
```

Estimated MVP peripheral LoC: ~3.5–4k hand-written + ~5k auto-ingested YAML.

## 9. Trace format, HW runner, `labwired compare`

### 9.1 Trace formats

- **VCD** via the `vcd` crate (MIT). Signal timelines; GTKWave / Surfer / sigrok viewers; `vcddiff`-friendly.
- **Perfetto JSON** hand-emitted. Event timelines; `ui.perfetto.dev` viewer.

### 9.2 VCD signal hierarchy (both sim and HW emit the same tree)

```
sim/
  cpu0/   pc, ccount, windowbase, windowstart, ps, intlevel, vecbase, exccause
  cpu1/   ... same ...
  bus/    addr, data, width, rw, cpu_id
  extmem/ icache_mmu_hit, translated_phys
  gpio/   pin0 .. pin21
  uart0/  tx_byte, rx_byte
  usb_cdc/ tx_byte, rx_byte
  int/    pending[0..93], taken_by_cpu
  sys/    systimer_u0, systimer_u1, ccompare0/1/2_cpu0/1
  ext/    ws2812_decoded_rgb[0..n]
```

### 9.3 Perfetto event taxonomy

- Per-core slice tracks: one slice per function entry (CALL*/ENTRY detected), coloured by window depth.
- Instant events: interrupt dispatch, exception raise, window under/overflow, VECBASE write, CPENABLE transition, atomic op.
- Async tracks: UART TX, USB-CDC TX, WS2812 frame.
- Counter tracks: CCOUNT per core, SYSTIMER U0/U1, IRQ pending count.

### 9.4 Time alignment

Firmware emits `LWTRACE_SYNC` (8-byte magic + monotonic counter via RTT) in the first few instructions of `main`. Sim and HW-runner rebase their picosecond timestamps to this marker. Cross-diffs are directly comparable.

### 9.5 `crates/hw-trace/`

```
src/
  event.rs     — TraceEvent enum
  sink.rs      — TraceSink trait; sim live sink + file replay sink
  vcd.rs       — VcdWriter
  perfetto.rs  — PerfettoWriter
  sync.rs      — LWTRACE_SYNC + rebase
  diff.rs      — diff engine with tolerance windows
```

### 9.6 `crates/hw-runner/`

```
src/
  flash.rs     — espflash library: firmware + bootloader + partition table
  reset.rs     — OpenOCD reset, halt, release
  rtt.rs       — RTT up-channel reader (OpenOCD TCL)
  openocd.rs   — TCL bridge; shared with hw-oracle
  cdc.rs       — /dev/ttyACM0 byte capture with host-clock + skew correction
  sigrok.rs    — optional: sigrok-cli subprocess for GPIO
  encoder.rs   — merge RTT + CDC + sigrok streams into shared hw-trace
  runner.rs    — orchestration
```

### 9.7 CLI

```
labwired compare  --system esp32s3-zero.yaml --firmware X.elf --duration 10s --out trace-diff/
labwired capture  --system esp32s3-zero.yaml --firmware X.elf --duration 10s --out golden.vcd
labwired diff     trace-diff/sim.vcd trace-diff/hw.vcd
labwired run      --system esp32s3-zero.yaml --firmware X.elf --trace out.vcd
```

### 9.8 Diff engine verdicts

Per-signal verdict ∈ {`exact`, `equivalent`, `mismatch`, `missing`}.

- **exact:** every transition bit-for-bit at same ps timestamp.
- **equivalent:** same value sequence, timestamps within tolerance window.
- **mismatch:** value sequences differ or out of order.
- **missing:** present on one side only.

Default tolerances (configurable per signal):

| Signal class | Value tolerance | Timestamp tolerance |
|---|---|---|
| `cpu*.pc`, `cpu*.windowbase/windowstart/ps/vecbase` | bit-exact | 0 |
| `bus.*` same-core | bit-exact | 0 |
| `bus.*` cross-core | bit-exact | ±5 cycles |
| `gpio.*` | bit-exact | ±1 bus clock |
| `uart0.tx_byte`, `usb_cdc.tx_byte` | bit-exact sequence | ±1 byte-time |
| `cpu*.ccount` | — | ±2 cycles (MVP), exact (v1.1) |

Per-test overrides: `#[hw_oracle_test(tolerance = "cpu0.pc=exact; cpu0.ccount=strict")]`.

### 9.9 CI integration

- Self-hosted runner with the S3-Zero attached runs `cargo test --features hw-oracle` on every PR.
- Sim-only regression: `labwired diff` against committed goldens for every fixture firmware. Runs without HW.
- Diff summary JSON uploaded as PR annotation. `mismatch` blocks merge; `equivalent` passes with a note; `missing` blocks unless explicitly one-sided.

### 9.10 Sync marker caveat

`LWTRACE_SYNC` requires firmware cooperation. Fixture firmwares include it via `esp32s3-fixture::labwired_runtime::init()`. Third-party firmware that doesn't include the marker can still be compared on GPIO + UART + USB-CDC alone but loses CPU-state alignment. Documented and accepted.

## 10. Fixture firmware

```
crates/esp32s3-fixture/
  src/lib.rs                     — labwired_trace! macro, LWTRACE_SYNC, panic handler, console routing
  src/bin/ci_isa_oracle.rs       — ISA oracle test runner (loads cases from embedded YAML)
  src/bin/ci_peripheral_probe.rs — peripheral register-probe harness

examples/esp32s3-blinky/         — WS2812 rainbow via bit-banged GPIO21 + SYSTIMER
examples/esp32s3-hello-world/    — UART0 + USB-CDC print
examples/esp32s3-gpio-interrupt/ — BOOT button → ISR → LED toggle
examples/esp32s3-timer-alarm/    — SYSTIMER alarm → ISR
examples/esp32s3-i2c-probe/      — I²C0 scan + tmp102 read
examples/esp32s3-spi-loopback/   — SPI2 MOSI→MISO
examples/esp32s3-smp-ping-pong/  — dual-core IPC via CPU_INTR_FROM_CPU_0
examples/esp32s3-fp-sqrt/        — FP sqrt + print
examples/esp32s3-embassy-led/    — Embassy async single-core
```

All fixtures depend on `esp32s3-fixture` for runtime glue. Each fixture ships with a committed `golden.vcd` + `golden.summary.json` captured from the physical board.

`esp-hal` is vendored at a pinned tag under `vendor/esp-hal-<tag>/` with MIT/Apache-2.0 attribution. Refresh is manual and reviewed.

## 11. Testing strategy

Six layers:

1. **Unit tests** — `#[cfg(test)]` in `crates/core`. Decoder tables, register-file rotation, SR plumbing, softfloat edge cases, bus routing. Every commit. No HW.
2. **Integration tests** — hand-written Xtensa asm snippets with end-state asserts. ~500 by completion. Every commit. No HW.
3. **Oracle tests** — `#[hw_oracle_test]` per §7. ~215 ISA + ~200 peripheral + ~50 SMP/FP. Gated `--features hw-oracle`. Self-hosted CI runner with S3-Zero attached.
4. **Golden-trace regression** — `labwired diff fixture.vcd golden.vcd`. Every commit, no HW.
5. **Property-based fuzz** — proptest: decoder round-trip, FP ops vs softfloat reference, windowed-reg invariants. Every commit.
6. **Nightly HW soak** — each fixture firmware 10+ minutes on HW and sim; full trace diff. Catches timing drift.

CI workflows:

- `.github/workflows/ci.yml` — fmt, clippy, unit, integration, fuzz, golden regression, fixture compile. PR gate. No HW.
- `.github/workflows/hw-oracle.yml` — self-hosted + S3-Zero. PR label `hw-test` or nightly. `cargo test --features hw-oracle`.
- `.github/workflows/nightly-soak.yml` — self-hosted, full soak + trace diff.

## 12. Schedule and milestones

Target: 16–22 weeks at ~1.0 FTE to full MVP including dual-core + FPU + HW-oracle CI green. Risk concentrated in M2 (windowed regs + exceptions) and M3 (ROM stub long tail). FPU work sits in M6.

| Milestone | Week | Deliverable |
|---|---|---|
| M1 | 3  | Decoder + base core executing hand-asm. Oracle harness (OpenOCD + proc-macro) online. |
| M2 | 5  | Windowed regs + exception/interrupt dispatch. Fibonacci asm returns. |
| M3 | 7  | Fast-path boot reaches `main`. `hello-world` prints via UART0 + USB-CDC. |
| M4 | 9  | Blinky + GPIO interrupt. First end-to-end `labwired compare` PASS verdict. |
| M5 | 11 | I²C0 + SPI2. tmp102 round-trip. |
| M6 | 13 | FPU softfloat + Boolean Regs + CPENABLE. FP-sqrt fixture. |
| M7 | 16 | Dual-core scheduler + inter-core IRQ. SMP ping-pong fixture. |
| M8 | 18 | Fixture suite complete. Nightly HW-oracle CI green. |
| M9 | 20 | Golden-trace regression stable. Release `labwired-core v0.12.0` with ESP32-S3-Zero as primary target. |
| M10 | 22 | Polish, docs, `docs/case_study_esp32s3.md` case study. |

## 13. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Windowed-register exception semantics (OF/UF handlers, SPILL_REGISTERS) have subtle hardware corner cases | High | High — boot hangs | Oracle tests for every window transition from day one. Week 5 budget assumes 2–3 days of debugging in that single area. |
| 16-bit / 24-bit instruction straddle across fetch boundary | Medium | High — random misdecode | Length-predecoder unit-tested with every 4-byte window permutation before integrating into CPU. |
| ROM stub long-tail (unknown stubs discovered during bringup) | High | Medium — slips M3 | Intercept at OpenOCD level on HW to log every BROM entry firmware calls; implement stubs incrementally. |
| OpenOCD Xtensa session flakiness on USB-JTAG | Medium | Medium — oracle CI noise | File-lock serialization; auto-retry on connection drop; nightly board power-cycle. |
| softfloat rounding-mode edge cases not matching LX7 | Low | Low — FP-fixture fails | Dedicated FP oracle bank; fall back to table-lookup for known-divergent cases. |
| Dual-core memory ordering violated by lazy scheduling | Medium | High — SMP hang | 16-insn quantum + event-driven reschedule; property fuzz test for Rust atomic sequences. |
| GDMA stubbed in MVP but some fixture needs real DMA | Low | Medium — fixture subset limited | MVP fixture list chosen to avoid DMA paths; revisit if a demo is forced. |
| Xtensa Rust toolchain (esp-rs/rust) version drift breaks fixture builds | Medium | Low | Pin toolchain in `rust-toolchain.toml`; vendor esp-hal at explicit tag. |
| Self-hosted CI runner with physical board becomes single point of failure | Medium | Medium | Second board kept as warm spare; oracle tests survive runner loss (sim-only CI still gates PRs). |

## 14. Open questions (none blocking)

All blocking design decisions are resolved. Items left for implementation-plan stage:

- Exact rust-toolchain.toml revision of the esp-rs Xtensa toolchain.
- Specific `esp-hal` tag to vendor.
- Whether `crates/hw-oracle` and `crates/hw-runner` share one OpenOCD process pool or two (probably one, finalized when building §7.2).
- Whether WS2812 decoder lives in `external_devices/ws2812.rs` (current plan) or in `crates/hw-trace` as a generic bit-time decoder reusable for other protocols.

## 15. References

### Primary (first authority)
- The physical Waveshare ESP32-S3-Zero board, via OpenOCD JTAG.

### Licensable reuse (dependencies)
- `esp-pacs` (MIT/Apache) — SVD source.
- `esp-hal` (MIT/Apache) — pinned-tag vendored fixtures.
- `espflash` (MIT/Apache) — host-side flashing.
- `vcd` crate (MIT) — trace emission.
- `softfloat` crate (MIT/Apache) — FP arithmetic.
- OpenOCD (GPL-2.0) — subprocess only.
- `sigrok-cli` (GPL-3.0) — subprocess only, optional.

### Reference only (not copied)
- ESP32-S3 Technical Reference Manual v1.x (Espressif).
- Cadence Xtensa LX ISA Reference Manual.
- Espressif QEMU fork, `esp-develop` branch — GPL-2.0, read-only for peripheral-semantics understanding.
- `xtensa-lx`, `xtensa-lx-rt` source — MIT/Apache but read as ground-truth spec for what the sim must support.
- Espressif `xtensa-isa-doc` — clean-room safe reference.
- Renode Xtensa notes (MIT) — peripheral-DSL design inspiration.

### Relevant in-repo files
- `docs/architecture.md` — current simulator architecture.
- `docs/declarative_peripherals.md`, `docs/declarative_registers.md` — declarative engine semantics.
- `docs/peripheral_development.md` — SVD ingestion workflow.
- `docs/plan.md` — roadmap including "golden reference" physical-board validation.
- `crates/core/src/peripherals/declarative.rs` — declarative engine; needs word-granular trigger support (prereq).
- `crates/core/src/decoder/riscv.rs` — style reference for the new Xtensa decoder.
- `crates/svd-ingestor/src/lib.rs` — SVD-to-YAML pipeline used for peripheral descriptors.
