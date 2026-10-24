# External components and model status

| Component | Selected function | Model / evidence | Current state |
|---|---|---|---|
| ESP32-C3-MINI-1U + external antenna | ESP-AT socket offload via UART4 | Deterministic UART protocol peer; independent physical RF/firmware capture | Shared behavioral peer instantiated through native UART4; reduced Linux protocol accepted; physical ESP firmware/RF unqualified |
| ADXL345 | I2C3 motion / tilt sensing | Existing [I2C kit](../../configs/devices/adxl345.yaml) and `test_adxl345_devid_and_axis_read`; synthetic XYZ inputs in g | Shared kit instantiated through native I2C3; reduced Linux joint scenario accepted; physical data/electrical qualification unavailable |
| Protected 1S 2000 mAh LiPo | Portable energy source | Pack datasheet and measured discharge / cutoff | Exact pack pending |
| BQ24074 | Charging and system power path | Vendor reference circuit; physical thermal / charging checks | Selected physical device unqualified; represented only by the reduced persistent power plant |
| TPS61023 | 5 V boost | Vendor reference circuit; transient and current checks | Selected physical device unqualified; represented only by the reduced persistent power plant |
| TPS62160 | 3.3 V buck from boosted 5 V | Vendor reference circuit; ESP radio-load transient checks | Selected physical device unqualified; represented only by the reduced persistent power plant |
| Reverse blocking / power mux | Isolate USB and carrier 5 V sources | Schematic analysis plus reverse-current measurements | Physical circuit/assembly qualification pending; canonical schematic and PCB exist |
| Low-battery shutdown / latch | Shutdown Linux and switch off independently | Threshold/hysteresis design and physical shutdown log | Physical circuit/assembly qualification pending; canonical schematic and PCB exist |
| MicroSD | Mini A vendor OS and local data | Boot image hash, card identity and crash/recovery checks | Physical setup pending |

The generic declared Mini A manifest still instantiates neither external kit. The separate [accepted reduced native Linux basket](../luckfox-pico-mini-a-linux/BASKET.md) attaches the same ADXL345 register kit and behavioral ESP-AT peer through actual I2C3/UART4 controllers on the LabWired SystemBus. The ESP32-C3 CPU/board models alone are not that serial peer, and the native peer executes no ESP firmware or RF. The same persistent reduced power plant carries battery, rails, latch/reset and time across fresh CPU/bus/component recovery; it is not a validated electrical model of each selected physical power device. [HARDWARE.md](HARDWARE.md) defines the selected power tree and signal routing; [REQUIRED_DOCS.md](REQUIRED_DOCS.md) points to the primary references.
