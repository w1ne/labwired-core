# External components and model status

| Component | Selected function | Planned model / evidence | Current state |
|---|---|---|---|
| ESP32-C3-MINI-1U + external antenna | ESP-AT socket offload via UART4 | Deterministic UART protocol peer; independent physical RF/firmware capture | Selected; not instantiated or validated |
| ADXL345 | I2C3 motion / tilt sensing | Existing [I2C kit](../../configs/devices/adxl345.yaml) and `test_adxl345_devid_and_axis_read`; synthetic XYZ inputs in g | Sensor model exists; RV1103 I2C3 connection and board scenario absent |
| Protected 1S 2000 mAh LiPo | Portable energy source | Pack datasheet and measured discharge / cutoff | Exact pack pending |
| BQ24074 | Charging and system power path | Vendor reference circuit; physical thermal / charging checks | Selected; no electrical model |
| TPS61023 | 5 V boost | Vendor reference circuit; transient and current checks | Selected; no electrical model |
| TPS62160 | 3.3 V buck from boosted 5 V | Vendor reference circuit; ESP radio-load transient checks | Selected; no electrical model |
| Reverse blocking / power mux | Isolate USB and carrier 5 V sources | Schematic analysis plus reverse-current measurements | Part / circuit pending |
| Low-battery shutdown / latch | Shutdown Linux and switch off independently | Threshold/hysteresis design and physical shutdown log | Part / circuit pending |
| MicroSD | Mini A vendor OS and local data | Boot image hash, card identity and crash/recovery checks | Physical setup pending |

The ADXL345 already has a reusable LabWired register model; it is not instantiated by the declared Mini A manifest. The existing ESP32-C3 CPU/board models do not constitute an ESP-AT serial protocol peer. No power device here has a validated electrical model. Existing MCU I2C/UART models do not by themselves describe RV1103 register semantics. [HARDWARE.md](HARDWARE.md) defines the selected power tree and signal routing; [REQUIRED_DOCS.md](REQUIRED_DOCS.md) points to the primary references.
