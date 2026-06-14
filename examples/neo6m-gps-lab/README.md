# NEO-6M GPS Lab

This firmware reads a u-blox NEO-6M GPS receiver over LabWired's simulated UART1
path on STM32F103. The modeled receiver streams NMEA sentences; the firmware
echoes parsed fixes. This is the UART-stream example (as opposed to the
query/response I²C and SPI sensor labs).

Run from the repo root:

```bash
cargo build -p neo6m-gps-lab --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- test --script examples/neo6m-gps-lab/io-smoke.yaml
```

Expected UART begins with:

```text
NEO-6M GPS Lab
Reading NMEA stream from UART1...
[GPS] ...
```

The GPS module attaches as a `uart_device` on UART1 (`device_type: neo6m-gps`
in `system.yaml`).
