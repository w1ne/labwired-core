# NUCLEO-U575ZI-Q Example (STM32U575ZI)

First STM32U5 target in LabWired. This example runs **three independent real
firmware stacks** on the modelled NUCLEO-U575ZI-Q:

1. `crates/firmware-stm32u575-demo` — bare-metal Rust smoke (`OK` over USART1)
2. `board_firmware/` — stock **STM32CubeU5 HAL**: MSI 4 MHz → PLL1 160 MHz,
   VOS1/SMPS, flash latency, ICACHE, USART1 VCP banner + LD1 blink loop
3. The fidelity matrices — **Arduino** (STM32 core) and **Zephyr**
   (`nucleo_u575zi_q`) builds, plus committed survival fixtures

Console: **USART1** PA9/PA10 @ 115200 8N1 (board VCP). LED: **PC7** (LD1,
Arduino `LED_BUILTIN`).

> Tier: **sim-validated** — SVD/RM0456-derived, no bench part, no silicon diff
> (Renode has no STM32U5 platform either). See
> [`VALIDATION.md`](VALIDATION.md) for the exact commands and captured
> evidence, and [`../../docs/boards/stm32u575.md`](../../docs/boards/stm32u575.md)
> for the model/honesty table.

## Quick start

Run from the repo root.

### 1. Rust smoke + io-smoke

```bash
cargo build -p firmware-stm32u575-demo --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test --script examples/nucleo-u575zi/io-smoke.yaml
```

Expected:

```
OK
PASS  2/2 checks · io-smoke · 64 steps · 0.00s
```

### 2. Vendor CubeU5 HAL firmware

Requires an STM32CubeU5 checkout (see
[`EXTERNAL_COMPONENTS.md`](EXTERNAL_COMPONENTS.md)) and `arm-none-eabi-gcc`:

```bash
make -C examples/nucleo-u575zi/board_firmware
cargo run -q -p labwired-cli -- \
  --firmware examples/nucleo-u575zi/board_firmware/build/u575_hal_smoke.elf \
  --system examples/nucleo-u575zi/system.yaml --max-steps 20000000
```

Expected (20M steps ≈ 0.14 s simulated; each blink costs 250 ms):

```
U575-HAL OK
BLINK 0 LD1=1
```

### 3. Fidelity matrices

```bash
cargo build -p labwired-cli --release

# Arduino matrix (L0-L4/L6/L7 pass; L5/L8 documented skips)
python3 validation/arduino-matrix/run_matrix.py --boards stm32u575

# Zephyr matrix (L0-L3)
PATH="$HOME/zephyrproject/.venv/bin:$PATH" \
  python3 validation/zephyr-matrix/run_matrix.py --boards stm32u575
```

### 4. Survival fixtures (PR gate)

```bash
cargo test -p labwired-core --test firmware_survival stm32u575 -- --nocapture
```

Runs the committed stock Zephyr `hello_world` ELF and the Arduino L0 sketch
ELF; both are hard-asserted on fixture presence.

## Files

- `system.yaml` — example system manifest (chip + board IO)
- `io-smoke.yaml`, `uart-smoke.yaml` — CLI test scripts
- `board_firmware/` — CubeU5 HAL app (`main.c`, `Makefile`, README, HAL conf)
- `VALIDATION.md` — full runbook + captured evidence (Tasks 3–8)
- `REQUIRED_DOCS.md` — SVD hash, DS13736/RM0456/UM2861, BSP, Zephyr docs
- `EXTERNAL_COMPONENTS.md` — no required components; CubeU5 checkout

## References

- Chip descriptor: [`../../configs/chips/stm32u575.yaml`](../../configs/chips/stm32u575.yaml)
- Board system: [`../../configs/systems/nucleo-u575zi.yaml`](../../configs/systems/nucleo-u575zi.yaml)
- Board doc: [`../../docs/boards/stm32u575.md`](../../docs/boards/stm32u575.md)
