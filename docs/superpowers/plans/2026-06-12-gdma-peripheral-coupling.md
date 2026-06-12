# GDMA Peripheral-Coupled Mode Implementation Plan (Slice 3A)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ESP32-S3 GDMA moves real bytes between memory descriptors and the UART/SPI/I2S peripherals (peripheral-coupled mode), so standard ESP-IDF DMA drivers run instead of receiving phantom EOFs.

**Architecture:** Add the missing `PERI_SEL` register pair per channel; in `tick_with_bus()`, coupled channels pump bytes between descriptor chains and the target peripheral THROUGH THE BUS (UART via its real MMIO FIFO; SPI via its DMA-enabled transaction path; I2S via a new sample-stream surface). Unmodeled peripheral ids keep today's auto-complete fallback (compatibility). EOF/interrupt semantics per direction follow the existing M2M wiring (sources 66–75).

**Tech Stack:** Rust (labwired-core), register-level integration tests (no cross-toolchain dependency in the gate; e2e firmware optional follow-up).

**Spec:** parent `labwired` repo `docs/superpowers/specs/2026-06-11-hw-substrate-sota-design.md`, Slice 3 ("GDMA peripheral-coupled mode"). LCD coupling explicitly deferred.

**CRITICAL — workspace:** ALL work in the core worktree `/home/andrii/projects/labwired/.worktrees/core-slice3-gdma` (branch `feat/gdma-peripheral-coupling`, tracks core origin/main). Another agent works in `/home/andrii/projects/labwired` and `/home/andrii/projects/labwired/core` — NEVER touch those. Commits: no Claude/AI/assistant references, no Co-Authored-By.

**Ground-truth anchors (read before each task):** `crates/core/src/peripherals/esp32s3/gdma.rs` (958 LoC: M2M walk at 339–397, auto-complete fallback at 294–326, irq emission 431–450, doc-comment contract 79–86), `uart.rs` (FIFO at offset 0x00, STATUS counts at 0x1C), `gpspi.rs` (W0..W15 + `SPI_CMD.USR` immediate-complete + storage-only `DMA_CONF` 0x30), `i2s.rs` (control surface only; `RXEOF_NUM` 0x64), and the M2M tests (gdma.rs 500–959) as the test-style template.

---

### Task 0: PERI_SEL ground truth (research gate, no production code)

**Files:**
- Create: `docs/esp32s3_gdma_peri_sel.md` (short note)

GDMA on real silicon binds channels to peripherals via `GDMA_IN_PERI_SEL_CHn_REG` / `GDMA_OUT_PERI_SEL_CHn_REG`. Before implementing, pin down: (a) the register OFFSETS within the per-channel block, and (b) the `PERI_IN_SEL`/`PERI_OUT_SEL` value encodings (which integer selects SPI2, SPI3, UHCI0/UART-DMA, I2S0, I2S1, LCD_CAM, AES, SHA, ADC, RMT).

- [ ] **Step 1:** Search the repo for existing evidence: `grep -rn "PERI_SEL\|peri_sel\|peri_in_sel" crates/ docs/ scripts/` — ROM-thunk or hw-oracle captures may already reference them. Also check any vendored ESP-IDF headers (`grep -rln "GDMA_IN_PERI_SEL" --include=*.h .` and `grep -rn "SOC_GDMA" crates/`).
- [ ] **Step 2:** If absent in-repo, derive from authoritative public sources (ESP32-S3 TRM chapter "GDMA Controller", or esp-idf `soc/esp32s3/include/soc/gdma_struct.h` + `gdma_channel.h` via WebFetch of the espressif GitHub raw files). Record in `docs/esp32s3_gdma_peri_sel.md`: offsets relative to the channel block (expected: IN_PERI_SEL at 0x40, OUT_PERI_SEL at 0xA0 within the 0xC0 stride — VERIFY, do not trust this parenthetical), the value→peripheral table, and the citation (URL + struct names). Wrong offsets here poison every later task — this is the one task where slowing down pays.
- [ ] **Step 3:** Cross-check against the existing register map constants in gdma.rs lines 99–115 (no collisions with already-modeled offsets). Commit: `docs(core): ESP32-S3 GDMA PERI_SEL register/value ground truth`

---

### Task 1: PERI_SEL registers + coupled-mode routing skeleton

**Files:**
- Modify: `crates/core/src/peripherals/esp32s3/gdma.rs`

TDD. Add `IN_PERI_SEL`/`OUT_PERI_SEL` registers (offsets from Task 0) as read/write storage on each direction's state; define

```rust
/// Peripheral targets GDMA can couple to. Values per Task 0 ground truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DmaPeripheral {
    Spi2, Spi3, Uhci0, I2s0, I2s1, LcdCam, Aes, Sha, Adc, Rmt, // adjust to the verified table
    Unknown(u32),
}
impl DmaPeripheral { fn from_sel(v: u32) -> Self { /* per table */ } }
```

Routing rule in the link-start paths (replacing the blanket auto-complete at lines 294–326): when `MEM_TRANS_EN` is clear, look up `DmaPeripheral::from_sel(peri_sel)`:
- **Coupled set** (`Uhci0`, `Spi2`, `Spi3`, `I2s0`, `I2s1`): mark the direction `pending_coupled` (new state) — byte movement happens in `tick_with_bus` (Tasks 2–4). Do NOT latch EOF at start.
- **Fallback set** (everything else, incl. `Unknown`): keep today's immediate auto-complete byte-for-byte (the doc comment at 79–86 must be updated to describe the new split). This preserves AES/SHA/ADC-using firmware behavior exactly.

Tests (extend the existing test module style): PERI_SEL read/write round-trip per channel/direction; coupled-set start does NOT latch EOF; fallback-set start still auto-completes (pin existing behavior with an explicit test so the compatibility promise is enforced); M2M tests untouched and green.

Run: `cargo test -p labwired-core gdma` → green; `cargo clippy -p labwired-core -- -D warnings`; `cargo fmt --all`.
Commit: `feat(core): GDMA PERI_SEL registers + coupled/fallback routing split`

---

### Task 2: UART (UHCI0) coupling — TX and RX byte movement

**Files:**
- Modify: `crates/core/src/peripherals/esp32s3/gdma.rs`
- Modify (only if needed): `crates/core/src/peripherals/esp32s3/uart.rs`

On real silicon UART-DMA flows through UHCI0 bridging GDMA↔UART FIFOs. We model the data path, not UHCI framing: a coupled OUT (TX) channel walks its descriptor chain (reuse `walk_out_chain`) and pushes each byte to the UART by `bus.write_u8(UART_FIFO_ADDR, byte)` — the UART's real MMIO FIFO write path, so serial output, interrupts, and STATUS counts behave identically to CPU writes. A coupled IN (RX) channel reads `STATUS.RXFIFO_CNT` via the bus, pops available bytes via `bus.read_u8(UART_FIFO_ADDR)`, and writes them into the IN descriptor chain (reuse `walk_in_chain` mechanics, but incremental: partial fills must NOT latch `IN_SUC_EOF` until the descriptor chain is satisfied or the UART idles — model: EOF when the current descriptor fills OR rx fifo empties after at least one byte with `IN_DONE` per completed descriptor; verify the exact EOF policy against how ESP-IDF's uart driver consumes it — `uart_read_bytes` waits on `IN_SUC_EOF`; document the chosen policy in the code comment).

Which UART instance: derive the UART base address from the UHCI0 binding (UHCI0 bridges to UART0 by default; if the repo's uart.rs models multiple instances, couple to UART0's base and document). Respect throughput realism minimally: move at most N bytes per tick (pick the value other peripherals use for batched ticking; a `const COUPLED_BYTES_PER_TICK` keeps it tunable).

Tests (register-level, in gdma.rs test module or a sibling integration test — follow where the M2M tests live):
- TX: program a descriptor with "HELLO", set OUT_PERI_SEL=UHCI0, start OUT link → after ticks, the UART's tx output (however uart.rs exposes transmitted bytes — read its tests for the accessor) contains "HELLO"; OUT_EOF latched only after the last byte entered the FIFO.
- RX: preload the UART rx fifo (uart.rs test helper or MMIO loopback — read how uart tests inject rx), program IN descriptor, start IN link → memory contains the bytes; IN_SUC_EOF + irq source 66+n raised; partial-fill does not EOF prematurely.
- Both with the interrupt enable mask set, assert `explicit_irqs` carries the right source ids.

Run gates as Task 1. Commit: `feat(core): GDMA↔UART coupled transfers via MMIO FIFO path`

---

### Task 3: GP-SPI (SPI2/SPI3) coupling

**Files:**
- Modify: `crates/core/src/peripherals/esp32s3/gpspi.rs`
- Modify: `crates/core/src/peripherals/esp32s3/gdma.rs`

Today `gpspi.rs` auto-completes a transaction on `SPI_CMD.USR` using the W0..W15 buffer; `DMA_CONF` is storage-only. ESP-IDF's SPI master driver in DMA mode: configures GDMA channel (PERI_SEL=SPI2/3), sets `DMA_CONF` enables, fills descriptors, kicks `USR`. Model: when `USR` is kicked with DMA tx/rx enabled in `DMA_CONF` (read the real bit names from the register doc in gpspi.rs or ESP-IDF spi_struct.h — record them), the SPI defers completion until GDMA supplies/consumes the data: GDMA's coupled OUT chain provides MOSI bytes (replacing W-buffer reads), coupled IN chain receives MISO bytes (the model's existing MISO fill policy: attached device response if a device is attached — gpspi has an attach() path used by the e-paper work — else 0xFF). On completion: SPI latches TRANS_DONE; GDMA latches OUT_EOF/IN_SUC_EOF.

Coupling mechanism: GDMA and SPI are separate peripherals on the bus; the byte handoff cannot go through MMIO (W buffer is the non-DMA path). Implement the smallest cross-peripheral contract that fits the existing architecture — read how the bus owner composes peripherals (`crates/core/src/bus/mod.rs`) and choose: (a) a shared `DmaEndpoint` trait object registered by base-address/peri-id that GDMA can resolve during `tick_with_bus` (mirrors how external device attach() works), or (b) a mailbox in the bus that SPI posts "transaction pending, need N bytes" requests to and GDMA services (mirrors the STM32 DmaRequest pattern at dma.rs:127–246). Prefer whichever pattern the codebase already uses for cross-peripheral signaling — investigate `dma_requests` in `PeripheralTickResult` first; if that mechanism can carry this, use it and do NOT invent a new trait. Document the decision in the module doc.

Tests: DMA-mode SPI3 transaction with an attached test device (reuse the e-paper test device pattern from the 93f6f9a8 work or gpspi tests): descriptor-fed MOSI bytes reach the device byte-for-byte; device-fed MISO bytes land in the IN descriptor buffer; TRANS_DONE + both EOFs ordered correctly; non-DMA (CPU W-buffer) transactions unchanged (regression-pin with existing tests).

Run gates. Commit: `feat(core): GDMA-coupled SPI2/3 transactions (DMA-mode master path)`

---

### Task 4: I2S coupling — sample streaming

**Files:**
- Modify: `crates/core/src/peripherals/esp32s3/i2s.rs`
- Modify: `crates/core/src/peripherals/esp32s3/gdma.rs`

I2S on S3 is DMA-only (no CPU FIFO) — this task makes I2S actually move data for the first time. Scope (minimal faithful): TX path: coupled OUT chain bytes stream into a new I2S sample sink — an internal bounded buffer drained at the configured sample rate is over-modeling; instead expose transmitted samples to tests/observability the same way UART exposes tx bytes (read how uart.rs surfaces output and mirror it; if there's a generic observable mechanism use it). RX path: a test-injectable sample source feeds the IN chain; `RXEOF_NUM` (i2s.rs 0x64) governs when IN_SUC_EOF latches (that's its documented purpose — wire it for real). TX EOF: when the descriptor chain drains. Start/stop respects the I2S TX/RX start bits already modeled in i2s.rs.

Tests: TX stream of a known pattern lands in the sample sink in order with OUT_EOF after chain completion; RX with RXEOF_NUM = N latches IN_SUC_EOF after exactly N bytes; I2S start-bit gating (no movement while stopped).

Run gates. Commit: `feat(core): GDMA-coupled I2S sample streaming (RXEOF_NUM honored)`

---

### Task 5: Contract docs, compatibility matrix, full verification

**Files:**
- Modify: `crates/core/src/peripherals/esp32s3/gdma.rs` (module doc 79–86 rewrite)
- Modify: the compatibility/支持 matrix doc (`docs/specs/compatibility_matrix.md` in the parent repo is OUT of this worktree — instead update core's own doc: grep `docs/` for the S3/GDMA support statements and update those in THIS repo)
- Modify: `CHANGELOG.md` (core repo conventions — read recent entries)

1. Rewrite the gdma.rs "What remains unimplemented" section: coupled = UART(UHCI0)/SPI2/SPI3/I2S0/I2S1 with real byte movement; fallback auto-complete = AES/SHA/ADC/RMT/LCD_CAM + unknown (explicit list); LCD_CAM deferred-by-design note.
2. Sweep for stale claims: `grep -rn "auto-complete\|peripheral-coupled\|non-m2m" crates/core/src/peripherals/esp32s3/ docs/` and update every statement that now lies.
3. Full verification (paste summaries): `cargo fmt --all --check && cargo clippy -p labwired-core -- -D warnings && cargo test -p labwired-core` (scoped package — workspace-wide may need cross-compiled fixtures: prebuild `cargo build -p firmware-hil-showcase --release --target thumbv7m-none-eabi` if running broader suites, per the known strict_onboarding requirement).
4. Confirm no regression in the existing S3 e2e tests (`cargo test -p labwired-core --test e2e_labwired_ereader` and any other S3 e2e — the e-paper CPU-driven path must be untouched).

Commits: `docs(core): GDMA coupled-mode contract + stale-claim sweep` (verification has no separate commit).

---

## Self-review notes

- Parent-spec coverage (Slice 3 GDMA item): UART/SPI/I2S coupling (Tasks 2–4), EOF/interrupt semantics per direction (each task's tests + existing source-id wiring), LCD deferred (Task 5 doc), compatibility preserved for non-coupled peripherals (Task 1 fallback + pinned test).
- Deliberately out: e2e cross-compiled DMA firmware fixtures (Xtensa toolchain dependency — register-level tests gate the logic; a firmware fixture is a fast-follow once a toolchain-built ELF can be committed like other fixtures); virtual netif (Slice 3B, separate plan); dual-core verification (3C).
- The riskiest unknowns are pinned to Task 0 (PERI_SEL ground truth) and the Task 3 coupling-mechanism decision (investigate-first instruction with two named candidate patterns from the codebase).
- No placeholder steps: every task names its test cases, files, gates, and commit message; implementation details that depend on unread code are framed as verify-then-adapt with exact anchors.
