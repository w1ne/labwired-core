# ESP32-P4 / Guition JC4880P443 — LabWired example

The Guition **JC4880P443** is a 4.3" 480x800 MIPI-DSI touch panel module built
around the **ESP32-P4** (16 MB PSRAM, 32 MB flash), running LVGL.

> ⚠️ **The panel is not modelled.** This example targets the **ESP32-P4 HP core
> and its UART0 console**, not the display. There is no MIPI-DSI controller model
> in the engine (see `KNOWN_LIMITATIONS.md`), so the panel and its touch
> controller are declared as unmodelled stubs. The module is deliberately **not**
> in the part catalog.

## Run the UART smoke

```bash
cargo build -p firmware-esp32p4-demo --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test --script examples/esp32p4-guition-jc4880p443/uart-smoke.yaml
```

Expected:

```text
OK
PASS  2/2 checks · uart-smoke · 200000 steps · 0.02s
```

## Files

| File | Purpose |
|------|---------|
| `uart-smoke.yaml` | The UART0 console smoke (asserts `OK`). |
| `system.yaml` | Example system manifest (chip + board I/O + console socket). |
| `KNOWN_LIMITATIONS.md` | What is deliberately not modelled, including the DSI gap. |
| `REQUIRED_DOCS.md` | Authoritative sources for the memory map and register bases. |

Chip descriptor: [`configs/chips/esp32p4.yaml`](../../configs/chips/esp32p4.yaml).
Board page: [`docs/boards/esp32p4-jc4880p443.md`](../../docs/boards/esp32p4-jc4880p443.md).
