# NUCLEO-F401RE Validation Runbook

Run all commands from `core/`.

## 1) Optional: ensure target installed

```bash
rustup target add thumbv7em-none-eabi
```

## 2) Build smoke firmware

```bash
cargo build -p firmware-f401-demo --release --target thumbv7em-none-eabi
```

## 3) Run the board UART + GPIO smoke

```bash
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-f401re/io-smoke.yaml \
  --output-dir out/nucleo-f401re/io-smoke \
  --no-uart-stdout
```

Pass criteria:
1. exit code is `0`
2. USART2 contains `NUCLEO_F401RE_OK`
3. GPIOA5 records at least eight output edges

## 4) Run direct simulation for PC/SP evidence

```bash
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7em-none-eabi/release/firmware-nucleo-f401re-smoke \
  --system configs/systems/nucleo-f401re.yaml \
  --max-steps 10000 \
  --json
```

## 5) Run unsupported-instruction audit

```bash
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7em-none-eabi/release/firmware-nucleo-f401re-smoke \
  --system configs/systems/nucleo-f401re.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/nucleo-f401re
```

Pass criteria:
1. script exits `0`
2. audit report exists at `out/unsupported-audit/nucleo-f401re/report.md`
