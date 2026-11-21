# PR-gated io-smoke receipt

Recorded 2026-11-21 10:05 CET (Europe/Berlin) from this tree with:

```sh
./examples/nucleo-l031k6/firmware/build.sh
./target/debug/labwired test --script examples/nucleo-l031k6/io-smoke.yaml \
  --output-dir out/nucleo-l031k6-smoke --no-uart-stdout
```

Result: `PASS  3/3 checks · io-smoke · 150000 steps`.
`uart.log` is the captured USART2 sink (`STM32L031K6_OK`).
Firmware sha256: `beb7dc7702c5f55af4faf431a111da846ca4805c5a975968d8d39be414afb608`.
