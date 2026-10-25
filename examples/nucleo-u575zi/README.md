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

The browser demo is available at
[app.labwired.com/?lab=nucleo-u575-star-catcher&run=1](https://app.labwired.com/?lab=nucleo-u575-star-catcher&run=1).
It opens the curated OLED game and starts its stock ELF in the Playground
simulator. The link does not connect to a physical NUCLEO-U575ZI-Q.

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

### 1a. U575 Star Catcher OLED game

This simulator-only example uses I2C1 (PB8/PB9) for an attached SSD1306. The
firmware configures B1's PC13 pin as an input before reading it. Press B1 to
move the catcher; the LEDs show the score and USART1 reports moves, catches and
misses. Its smoke injects a B1 press and asserts the `B1 MOVE` UART event as
well as the OLED output.

```bash
cargo build -p firmware-stm32u575-demo --bin nucleo-u575-star-catcher \
  --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test --script \
  examples/nucleo-u575zi/star-catcher-smoke.yaml
```

The smoke checks that firmware selected input mode on PC13 (`B1 INPUT READY`),
that an injected press produces `B1 MOVE` on USART1, that the OLED is nonblank,
and that execution reaches the 5,000,000-step limit. See
[`EXTERNAL_COMPONENTS.md`](EXTERNAL_COMPONENTS.md) for the virtual display and
physical acceptance boundary.

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

- `system.yaml` — bare-board system manifest (chip + board IO)
- `star-catcher-system.yaml` — system manifest with the simulated SSD1306
- `io-smoke.yaml`, `uart-smoke.yaml`, `star-catcher-smoke.yaml` — CLI scripts
- `../../crates/firmware-stm32u575-demo/src/star_catcher.rs` — register-level OLED game
- `board_firmware/` — CubeU5 HAL app (`main.c`, `Makefile`, README, HAL conf)
- `VALIDATION.md` — runbook + captured evidence (Tasks 3–8 and current addenda)
- `REQUIRED_DOCS.md` — SVD hash, DS13737/RM0456/UM2861, SSD1306 reference, BSP, Zephyr docs
- `EXTERNAL_COMPONENTS.md` — simulated OLED, model boundary, CubeU5 checkout

## References

- Chip descriptor: [`../../configs/chips/stm32u575.yaml`](../../configs/chips/stm32u575.yaml)
- Board system: [`../../configs/systems/nucleo-u575zi.yaml`](../../configs/systems/nucleo-u575zi.yaml)
- Board doc: [`../../docs/boards/stm32u575.md`](../../docs/boards/stm32u575.md)
