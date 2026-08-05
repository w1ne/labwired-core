# Pico 2 (RP2350) Validation Runbook

Run all commands from `core/`.

## 0) Firmware rationale

The smoke firmware is `firmware-rp2350-demo`: bare-metal Cortex-M33
(`thumbv8m.main-none-eabi` + `cortex-m-rt`), linked at XIP flash
`0x10000000` with RAM at `0x20000000`. It writes `RP2350_SMOKE_OK\n` straight
to UART0 DR (`0x40070000` — the RP2350 PL011 base, not the RP2040's
`0x40034000`). What this proves: the RP2350 descriptor boots a Thumb ELF,
the vector table at flash base is accepted (`reset_vector_offset: 0`), and
UART0 on the RP2350 address map sinks bytes. What it cannot prove:
PIO v2, dual-core/SIO FIFO, TrustZone, real bootrom/IMAGE_DEF, USB, HSTX,
or silicon register parity — this chip is NOT_SHIPPED in the ratchet.

**Build note:** cortex-m-rt firmware must be linked with `-Tlink.x`
(`RUSTFLAGS="-C link-arg=-Tlink.x"` or the crate's `.cargo/config.toml`).
Without it the ELF has no `.text` / vector table and the loader rejects
the image.

## 1) Build smoke firmware

```bash
rustup target add thumbv8m.main-none-eabi   # once
RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-rp2350-demo \
  --release --target thumbv8m.main-none-eabi
```

## 2) Run deterministic UART smoke

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/pico2/uart-smoke.yaml \
  --output-dir out/pico2/uart-smoke \
  --no-uart-stdout
```

Pass criteria: exit `0`, UART contains `RP2350_SMOKE_OK`.

## 3) Run unsupported-instruction audit

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-rp2350-demo \
  --system examples/pico2/system.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/pico2
```

Pass criteria: exit `0`, report exists, `unsupported_total: 0`.

## Validation record

- 2026-08-05: UART smoke exit 0 (`RP2350_SMOKE_OK` received) on the
  board-pack RP2350 branch after fixing the firmware link (`-Tlink.x`);
  clkrst profile unit tests green (`rp2350_clkrst_profile`).
