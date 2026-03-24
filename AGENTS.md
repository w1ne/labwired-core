# AGENTS.md

Short operating notes for agents working in `labwired`.

## Read First

- `README.md`
- `DEVELOPMENT.md`
- `core/README.md`
- `core/docs/index.md`

Read extra docs only when the task needs them.

## Repo Map

- `core/`: simulator engine, CLI, configs, firmware fixtures, tests.
- `vscode/`: VS Code extension work.
- `docs/`: internal docs, strategy, runbooks.
- `marketing/`: external-facing material.

## Working Rules

1. Ground the task in something real: a user request, issue, tagged repo task, or specific file.
2. Check paths before editing.
3. Keep changes scoped.
4. Prefer existing docs/scripts over inventing new flow.
5. Validate the touched area before claiming completion.
6. Report exact commands run and concrete evidence.

If the task is unclear or not reproducible, stop and say so.

## Standard Commands

Core build/test/lint from repo root:

```bash
cd core
EXCLUDES="--exclude firmware-armv6m-hello --exclude firmware-stm32f103-blinky --exclude firmware-stm32f103-uart --exclude firmware-armv6m-ci-fixture --exclude firmware-armv7m-benchmark --exclude firmware-f401-demo --exclude firmware-h563-demo --exclude firmware-h563-fullchip-demo --exclude firmware-h563-io-demo --exclude firmware-hil-showcase --exclude firmware-nrf52832-demo --exclude firmware-rp2040-pio-onboarding --exclude firmware-rv32i-ci-fixture --exclude firmware-rv32i-hello"
cargo build --workspace $EXCLUDES
cargo test --workspace $EXCLUDES
cargo clippy --workspace $EXCLUDES -- -D warnings
cargo fmt --all -- --check
```

Run simulator:

```bash
cd core
cargo run -p labwired-cli -- --firmware path/to/firmware.elf --system path/to/system.yaml
```

VS Code extension:

```bash
cd vscode
npm install
npm run compile
```

## Board Onboarding

For new MCU/board work, read `core/docs/board_onboarding_playbook.md` first.

Minimum expected deliverables:

1. `core/configs/chips/<chip>.yaml`
2. `core/configs/systems/<board>.yaml`
3. smoke firmware crate
4. `core/examples/<board>/system.yaml`
5. `core/examples/<board>/README.md`
6. `core/examples/<board>/REQUIRED_DOCS.md`
7. `core/examples/<board>/EXTERNAL_COMPONENTS.md`
8. `core/examples/<board>/VALIDATION.md`

Minimum validation from `core/`:

```bash
cargo test -p labwired-core <new_or_updated_test_name> -- --nocapture
cargo build -p <firmware-demo-crate> --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- \
  --firmware target/thumbv7m-none-eabi/release/<firmware-demo-crate> \
  --system examples/<board>/system.yaml \
  --max-steps 32
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7m-none-eabi/release/<firmware-demo-crate> \
  --system configs/systems/<board>.yaml \
  --max-steps 200000 \
  --out-dir out/unsupported-audit/<board>
```

Do not mark board onboarding complete without runnable example docs, passing validation, UART smoke output, and an unsupported-instruction audit report.
