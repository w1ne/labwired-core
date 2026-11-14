# Arduino Nano R4 (ABX00142)

Renesas RA4M1 (R7FA4M1AB) in the Nano form factor. Current product: SKU
ABX00142 is on sale and the RA4M1 is Active with longevity to 2041
(`evidence/vendor-status.md`, read 2026-11-14).

System: `configs/systems/arduino-nano-r4.yaml` over `configs/chips/ra4m1.yaml`.

## What boots and passes

The stock ArduinoCore-renesas sketch in `firmware/src/main.cpp` (PlatformIO
`renesas-ra` 1.9.0, board `nano_r4`, framework 1.6.0) boots through the full
core `init()`, including TinyUSB `dcd_init`. It then prints on `Serial1`
(SCI2, D1 = P302) and toggles `LED_BUILTIN` (P204).

```bash
labwired test --script examples/arduino-nano-r4/smoke.yaml \
  --output-dir out/arduino-nano-r4 --no-uart-stdout
```

Result (`evidence/run/`): **PASS 3/3**, 1,220,000 steps. UART:

```
NANO_R4_BOOT
NANO_R4_OK blink=2 LED=P204 UART=SCI2
```

## Model change this board needed

The Nano R4 uses native USB. TinyUSB `dcd_init` sets USBFS `SYSCFG.SCKE` and
polls until the bit reads back. The USBFS window was a read-zero stub, so the
core hung there. It is now a declarative register file
(`configs/peripherals/ra4m1/usbfs.yaml`): registers latch, nothing else.
`INTSTS0.VBSTS` reads 0, so the core sees no cable and does not connect.

Negative control (`evidence/negative-control/`): the same ELF on the old
read-zero stub fails **0/3** and is still at PC 0x7ff0 (`dcd_init` SCKE poll)
after 5,000,000 steps.

## Not claimed

- Native USB CDC `Serial` — no USB protocol or host is modelled.
- RGB LED (P409/P410/P411), Qwiic connector (P400/P401), 3V3 enable (P500).
- Analog front end, supply and brown-out behaviour. No physical-board check.

## Copy-paste sketch

```cpp
void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial1.begin(115200);          // D0/D1 header, not USB
  Serial1.println("NANO_R4_BOOT");
}
void loop() {
  digitalWrite(LED_BUILTIN, HIGH); delay(5);
  digitalWrite(LED_BUILTIN, LOW);  delay(5);
}
```
