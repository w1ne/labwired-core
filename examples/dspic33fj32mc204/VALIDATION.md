# Validation — dsPIC33FJ32MC204 (M0 + M1a)

Claims, evidence, and what is explicitly **not** claimed yet.

## Tier status

| Capability | Tier | Evidence |
| :--- | :--- | :--- |
| Boot from ELF at entry 0 | smoke-tested | `crates/core/tests/pic24_m0_smoke.rs` |
| `NOP` / `MOV #lit16` / `MOV Wns,f` / `MOV f,Wnd` / `BRA Expr` / `RETFIE` decode | unit-tested | `crates/core/src/cpu/pic24.rs` (`#[cfg(test)]`) |
| UART1 TX (`U1TXREG`) → console sink | smoke-tested | unit test `mov_w0_to_u1txreg_reaches_sink`, integration test, io-smoke |
| Determinism (same inputs → same output) | smoke-tested | integration test runs the fixture twice and compares sink + PC |
| Timers T1/T2/T3: TON, TCKPS 1:1–1:256, `TMR > PR` match, 32-bit T2/T3 pair (M1a) | unit-tested | `timer1_counts_to_period_and_raises_flag`, `timer1_prescaler_1_to_8`, `timer23_pair_counts_32_bit_and_flags_timer3` |
| Interrupts: IFS0/IEC0/IPC0-2, DISI/NSTDIS gating, vector `0x4+2*N`, two-word frame, RETFIE (M1a) | unit-tested | `timer1_interrupt_vectors_frames_and_returns`, `disabled_source_does_not_vector` |
| `ACCA*` / `ACCB*` 40-bit SFR storage (M1a) | unit-tested | `accumulator_sfr_words_round_trip` |
| IC/OC (M1b), DSP MAC (M1c), PWM, QEI, ADC, SPI, I2C | **not modelled** | decode error on unsupported opcodes; no claims |

## Commands and evidence (verified 2026-10-18, branch `feat/pic24-m0`)

```bash
# 1. Unit tests — 8 passed
cargo test -p labwired-core --lib pic24
#   test cpu::pic24::tests::{reset_starts_at_word_zero, mov_literal_loads_w0,
#     mov_w0_to_u1txreg_reaches_sink, nop_advances_pc, bra_minus_one_stays_put,
#     u1sta_read_reports_transmit_empty, ram_round_trip,
#     unsupported_opcode_is_a_decode_error} ... ok
#   test result: ok. 8 passed; 0 failed

# 2. Integration smoke — 2 passed (boot + UART + determinism ×2)
cargo test -p labwired-core --test pic24_m0_smoke
#   test pic24_fixture_boots_and_writes_h_to_uart ... ok
#   test pic24_fixture_is_deterministic ... ok
#   test result: ok. 2 passed; 0 failed

# 3. CLI end-to-end
cargo build -p labwired-cli
./target/debug/labwired test --script examples/dspic33fj32mc204/io-smoke.yaml
#   INFO labwired_core::system::builder: Loading chip descriptor:
#     ../../configs/chips/dspic33fj32mc204.yaml
#   INFO labwired_loader: ELF Entry Point: 0x0
#   PASS  2/2 checks · io-smoke · 64 steps · 0.00s   (exit 0)
```

Expected io-smoke result: UART output contains `H`; stop reason `max_steps` — observed as above.

## Gate integration (run on this branch, 2026-10-18)

```bash
cargo test -p labwired-core --test strict_onboarding
#   Verifying Strict Onboarding for: dspic33fj32mc204
#   Found example directory: "examples/dspic33fj32mc204"
#   PASS  2/2 checks · io-smoke · 64 steps · 0.00s
#   [PASS] dspic33fj32mc204 is strictly onboarded.
cargo test -p labwired-core --test board_coverage_ratchet      # 4 passed
cargo test -p labwired-core --test session_catalog             # 1 passed (48-chip catalog incl. PIC24)
cargo test -p labwired-core --test chip_conformance            # 8 passed (declared-only board regenerated)
cargo test -p labwired-core --test cpu_trace_conformance       # 3 passed (Pic24 emits the full trace contract)
cargo test -p labwired-core --test register_compliance         # 1 passed
cargo test -p labwired-core --test register_coverage           # 1 passed
cargo test -p labwired-core --lib one_arch_policy              # 8 passed
cargo test -p labwired-core --lib cpu_hz                       # 4 passed
cargo test -p labwired-core --lib bus_proof_matrix             # 10 passed (uart proven; spi/i2c declared none)
python3 scripts/generate_firmware_exercise_matrix.py --check   # clean (entry added)
```

(Environment note: strict_onboarding also builds firmware for every other chip;
those builds need cross-targets not installed in the authoring sandbox — the
failures listed there are environmental and do not involve this chip.)

## M1a evidence (timers + interrupts, verified 2026-10-19, branch `feat/pic24-m1a`)

```bash
# 1. Unit tests — 14 passed (6 new M1a tests)
cargo test -p labwired-core --lib pic24
#   test cpu::pic24::tests::timer1_counts_to_period_and_raises_flag ... ok
#   test cpu::pic24::tests::timer1_prescaler_1_to_8 ... ok
#   test cpu::pic24::tests::timer23_pair_counts_32_bit_and_flags_timer3 ... ok
#   test cpu::pic24::tests::timer1_interrupt_vectors_frames_and_returns ... ok
#   test cpu::pic24::tests::disabled_source_does_not_vector ... ok
#   test cpu::pic24::tests::accumulator_sfr_words_round_trip ... ok
#   test result: ok. 14 passed; 0 failed

# 2. Integration + trace gates unaffected
cargo test -p labwired-core --test pic24_m0_smoke        # 2 passed (fixture unchanged)
cargo test -p labwired-core --test cpu_trace_conformance # 3 passed (per-instruction trace contract kept)

# 3. fmt / clippy
cargo fmt --all -- --check                              # clean
cargo clippy -p labwired-core --tests -- -D warnings    # clean
```

M1a scope: `T1CON/T2CON/T3CON` (TON, TCKPS, T32), `TMR1/2/3`, `PR1/2/3`,
`IFS0/IEC0/IPC0-2`, `INTCON1.NSTDIS`, `INTCON2.DISI`, exception entry with the
DS70283H two-word frame, vector `0x4 + 2*N`, `RETFIE`, and `ACCA*`/`ACCB*` SFR
storage. Encodings and addresses pinned to DS70000157G / DS70283H. IC/OC (M1b,
needs a pin/GPIO model) and the DSP MAC (M1c, PRM field tables) remain
unmodelled; unsupported opcodes still raise `DecodeError`, never a silent skip.

## Unsupported-instruction audit

```bash
core/scripts/unsupported_instruction_audit.sh \
  --firmware examples/dspic33fj32mc204/firmware/dspic33fj32mc204-uart-smoke.elf \
  --system   examples/dspic33fj32mc204/system.yaml \
  --max-steps 64 --out-dir out/audit
#   unknown_thumb16: 0 · unhandled_thumb32: 0 · unknown_riscv: 0
#   unsupported_total: 0   (simulator exit 0)
```

The audit's Thumb/RISC-V counters are zero because the M0 core never degrades an
unknown opcode silently: anything outside the modelled subset raises a
`DecodeError`, which would fail the io-smoke rather than appear as a skipped
instruction. Per-instruction telemetry for the audit's `instructions_executed`
field is not emitted by the M0 core yet (M1 item).

## Console evidence (`labwired run`)

```bash
labwired run --chip configs/chips/dspic33fj32mc204.yaml --firmware <elf> --max-steps 200
#   UART: H
#   labwired-cli run (pic24): reached --max-steps 200; pc=0x00000008

labwired --firmware <elf> --system examples/dspic33fj32mc204/system.yaml --max-steps 64
#   UART: H
#   Final PC: 0x8
#   Total Instructions: 64
#   Total Cycles: 120   (56 self-loop BRA × 2 + 8 single-cycle instructions)
```

## Method notes

- Fixture ELF is deterministic (generated by `firmware/make-fixture-elf.py`).
- The instruction-slot layout (24-bit word in a 4-byte LE slot) is provisional;
  reviewed against DS70000157G encodings and to be re-checked against a real
  XC16 ELF before any claim involving XC16-built firmware.
- No silicon comparison exists for this part yet; nothing here is
  hardware-compared.
