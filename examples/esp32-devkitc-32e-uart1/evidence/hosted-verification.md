# Hosted UART1 TX verification

On 2026-11-09, `labwired_verify` ran the UART1 transmit sequence on its
catalogued `esp32` board and returned `status: model_verified` for this oracle:

```json
{"gpio_edges":[{"pin":"GPIO16","min_edges":32}]}
```

The verifier observed 87 GPIO16 transitions and stopped after the assertion
passed. A serial clause is not available on the hosted classic-ESP32 profile:
it returned `SERIAL_NOT_OBSERVABLE` because that profile stubs
`HardwareSerial::write`. The successful hosted check therefore asserts the TX
waveform directly. Its only reported gap is that the MCU has no routed power
net and assumes an ideal 3.3 V rail; brown-out and power-on reset are not
simulated. The repository-local board smoke and complete firmware hash are in
the adjacent `simulation-output/result.json`.
