# LabWired GPIO oracle receipt

Run on 2026-11-03 against LabWired's catalog `rpi-pico` RP2040 board
profile. The catalog does not expose a distinct Pico W board profile, so this
is an oracle check of the shared RP2040 GP0 path, not of the CYW43439 or the
Pico W system descriptor itself.

- Oracle: `GP0` recorded at least 8 GPIO edges.
- Result: `status: model_verified`; 15 GP0 transitions were observed within
  the 20-million-step budget.
- Run stopped when the oracle passed.
- Non-blocking model gaps: the twin does not track PLL reconfiguration, and
  the board has no routed power net, so VDD is assumed ideal and power-on / brown-out
  reset are not simulated.

The oracle sketch was:

```cpp
void setup() { pinMode(0, OUTPUT); }
void loop() {
  digitalWrite(0, HIGH);
  delayMicroseconds(100);
  digitalWrite(0, LOW);
  delayMicroseconds(100);
}
```

The Pico W-specific `raspberry-pi-pico-w` system descriptor is separately
loaded and smoke-tested by the committed local engine evidence in this folder.
