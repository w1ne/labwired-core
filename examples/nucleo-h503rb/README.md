# NUCLEO-H503RB quick start

## Demand rank

STM32 Nucleo/Discovery kits appear in **23% of main projects** in the demand
survey. NUCLEO-H503RB is a current, in-stock example in that board category.
The H503 is an H5 family extension; this is a board coverage pick, not a newly
ranked MCU model. ST's live availability record is in
[vendor-status.md](vendor-status.md).

## Copy-paste smoke

The no-std Rust smoke prints `H503_USART3_OK` through the default STLINK-V3EC
VCP (USART3 on PA3/PA4) and toggles LD2 on PA5. Run it from the repository root:

```bash
cargo build -p firmware-nucleo-h503rb-smoke --release --target thumbv8m.main-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-h503rb/io-smoke.yaml \
  --output-dir out/nucleo-h503rb/io-smoke --no-uart-stdout
```

The smoke requires the USART3 marker and at least eight PA5 edges. The narrow
profile includes the H5 RCC, GPIOA/GPIOC, USART3, SysTick, LD2, B1, and the
default USART3 console selection. The engine captures USART3 output from its
register model; it does not simulate the ST-LINK VCP bridge or PA3/PA4 pad
waveform. USB, crypto, timers, other board peripherals, and the remaining board
connectors stay outside this profile.

The locally generated smoke logs and structured result are committed under
[`evidence/`](evidence/). No physical H503 board capture is claimed.
