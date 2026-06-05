# ESP32-S3 Full Chip Model — Design

**Date:** 2026-06-05
**Status:** Draft for review
**Scope:** ESP32-S3 (Xtensa LX7) only. Classic ESP32 / generic Xtensa / ARM / RISC-V paths are out of scope and untouched.

## 1. Goal & definition of done

Make the ESP32-S3 a **faithful-by-default, register-complete chip model**:

1. **Faithful by default.** The real-ROM path runs automatically with no env-var
   dance. The thunk path is retained only as an explicit, clearly-labelled
   *graceful fallback* when the ROM blob cannot be located.
2. **Register-complete + state-machine-faithful (primary objective).** Every
   peripheral models **every register and every field** with correct access
   semantics (RW / RO / W1C / W1S / self-clearing / read-as-zero), correct reset
   values, and **all of its documented internal state machines** — not merely the
   subset that the firmware exercised so far.

"0-thunk boot of real firmware" is *necessary but not sufficient*. A peripheral
is only "done" when its full register map and FSMs are modelled and validated.

### Non-goals

- Radio PHY/MAC (WiFi, BLE) — explicitly out of scope.
- Register-completeness for the classic ESP32 or other architectures.
- Cycle-exact timing (functional FSM fidelity, not pipeline timing).

## 2. The completeness bar (per-peripheral definition of done)

A peripheral meets the bar when **all** of the following hold, tracked in the
coverage matrix (§4):

1. **Register map complete.** Every register and field present in the ESP32-S3
   SVD (§3 oracle) is modelled: correct offset, width, reset value, and access
   type. No silent "read-as-zero / accept-and-ignore" for registers a real
   driver reads or writes.
2. **Access semantics correct.** W1C (write-1-to-clear), W1S, self-clearing
   trigger bits (e.g. `CONF_UPGATE`, FIFO-reset, `TRANS_START`), RO status
   fields, and latched fields behave as silicon.
3. **Internal state machine(s) faithful.** The peripheral's documented FSMs are
   implemented and observable through the registers — e.g. I2C command-list
   engine, SPI master transaction FSM, SAR-ADC conversion FSM, RMT TX/RX
   encoder, LEDC/MCPWM counter+compare, UART FIFO occupancy, timer counter/alarm.
4. **Interrupts complete.** Every `INT_RAW`/`INT_ENA`/`INT_ST`/`INT_CLR` bit is
   modelled with correct set/clear behaviour, and the peripheral asserts the
   **correct interrupt-matrix source ID** (the `ets_isr_source_t` ordinal — the
   class of bug fixed for I2C0: source 42, not 49).
5. **Validated three ways:** SVD register-coverage diff is green (§4);
   register-level unit tests cover the FSM transitions and access semantics; and
   a real ESP-IDF/Arduino driver drives it to success over the faithful
   0-thunk `--rom-boot` path. HW-oracle trace where a physical board is on hand.

## 3. Completeness oracle: the ESP32-S3 SVD + PAC

Authoritative register sources, already present:

- `~/.platformio/platforms/espressif32/misc/svd/esp32s3.svd` — every peripheral,
  register, field, reset value, access type.
- `esp32s3` PAC (crates.io 0.35.2) — cross-check for field semantics.
- Repo already ships `crates/svd-ingestor/` (SVD → register-map parser).

The model is **measured against the SVD**, so "all registers modelled" is an
automatable check, not a judgement call.

## 4. Coverage tooling (the tracking artifact)

New dev tool: `xtask svd-coverage --chip esp32s3` (or a `cargo test` lane):

- Parse the SVD into the authoritative `{peripheral → registers → fields}` set.
- Introspect each modelled peripheral's known offsets/fields (via a small
  `ModelledRegisters` descriptor each peripheral exposes, or by parsing its
  `REG_*` constants + a declared field table).
- Emit a **coverage matrix**: per peripheral, `modelled / total` registers and
  fields, with a list of unmodelled or semantically-divergent registers.
- A ratchet: the matrix is committed; CI fails if coverage regresses. Peripherals
  graduate to "complete" when 100% with FSM tests attached.

This matrix is the living definition of "full chip model" — the chip is full
when every in-scope peripheral row is green.

## 5. Architecture / components

### 5.1 ROM auto-provisioning (`boot::esp32s3::rom_provision`)
At `configure_xtensa_esp32s3`, if no explicit `LABWIRED_ESP32S3_ROM` override:
1. **Discover** the toolchain ROM ELF (search
   `~/.platformio/tools/tool-esp-rom-elfs/esp32s3_rev0_rom.elf`, `$IDF_PATH`,
   an env hint).
2. **Extract** flat IROM (`0x4000_0000`, 384 KiB) and DROM (`0x3FF0_0000`,
   128 KiB) images in Rust — a port of `scripts/make_esp32s3_rom_bins.py`,
   including the `.data` copy-source reconstruction and `p_paddr`-based IROM
   layout (the working extraction does more than a naive PT_LOAD-by-vaddr copy).
3. **Cache** under a state dir, keyed on the ELF content hash.
4. **Load** as today. The ROM blob is never vendored (Espressif copyright); it is
   read from the user's own installed toolchain.

If discovery fails → fall back to the thunk **harness** with a one-line notice:
"ESP32-S3 ROM not found; running in degraded harness mode — install the ESP
toolchain for faithful simulation."

### 5.2 Faithful-default switch + cleanup
- `configure_xtensa_esp32s3` defaults to the faithful path when the blob
  resolves; thunk harness retained as fallback (shared `RomThunkBank`, untouched
  for classic ESP32).
- Delete `wifi_thunks.rs` — confirmed dead code (no references but its `mod`).

### 5.3 Faithful-mode gate + telemetry
- The model reports its boot mode (`faithful` | `harness`) and a count of
  `BREAK 1,14` thunk dispatches.
- Test helper `assert_faithful_zero_thunks()` — real ROM loaded, 0 thunk
  dispatches — used by every per-peripheral boot test.

### 5.4 Per-peripheral register+FSM model (the bulk of the work)
Each peripheral slice brings one peripheral to the §2 bar. A shared pattern:
a documented register/field table (checked by the coverage tool), the FSM(s)
in `tick()`/`write_u32`, complete interrupt handling, and register-level tests.

## 6. Slices / sequence

**Infrastructure (enables everything):**
- **S1 — ROM auto-provisioning** (§5.1).
- **S2 — Faithful default + delete `wifi_thunks`** (§5.2).
- **S3 — Coverage tool + faithful-mode gate** (§4, §5.3): SVD coverage matrix
  committed; per-peripheral boot-test harness in place.

**Per-peripheral completeness (one slice each, independently shippable):**
Ordered by SpiceDispenser relevance, then breadth. Each = bring the peripheral to
the §2 bar (all registers + FSMs + interrupts + 3-way validation):
- **S4 — I2C0/I2C1**: already 0-thunk boots; finish register/field completeness +
  FSM tests to 100% (close the remaining `accept-and-ignore` regs).
- **S5 — GP-SPI2/SPI3** (drives e-paper SSD16xx; SpiceDispenser-adjacent).
- **S6 — LEDC + MCPWM** (servo/PWM).
- **S7 — SAR-ADC**.
- **S8 — RMT**.
- **S9 — Timer Group + SYSTIMER** (audit to completeness; partially modelled).
- **S10 — UART/USB-Serial-JTAG**.
- **S11+ — remaining TRM peripherals** (I2S, TWAI, PCNT, SDMMC, GDMA, crypto…)
  driven by the coverage matrix until every in-scope row is green.

The peripheral set beyond S4–S8 is driven by the coverage matrix, not guesswork.

## 7. Testing & validation strategy

- **SVD coverage diff** (CI ratchet) — register/field completeness.
- **Register-level unit tests** — FSM transitions + access semantics per
  peripheral (extend the existing `#[cfg(test)]` blocks).
- **Faithful 0-thunk boot tests** — real ESP32-S3 firmware per peripheral over
  `--rom-boot`, asserting driver success + `assert_faithful_zero_thunks()`.
  Gated on the toolchain ROM ELF being present (like existing e2e firmware tests).
- **HW oracle** — register-trace diff vs a physical ESP32-S3 over USB-JTAG where
  available (the existing oracle lane).

## 8. Risks & open questions

- **Register introspection mechanism** for the coverage tool: each peripheral
  must expose its modelled register/field set. Cheapest is a declared table per
  peripheral; needs a small trait. (Decide in S3.)
- **Firmware availability** for breadth peripherals — need real S3 sketches
  exercising SPI/ADC/RMT/PWM. Existing `examples/esp32s3-*` cover some; others
  may need a small Arduino/IDF sketch each.
- **Effort** — full register completeness across ~30 peripherals is large; the
  coverage matrix makes progress legible and lets us ship peripheral-by-peripheral.
- **FSM depth ceiling** — functional fidelity, not cycle-exact timing (non-goal).
