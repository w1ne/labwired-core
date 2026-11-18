# Nokia 5110 Breakout

This firmware runs Breakout on a **NUCLEO-L476RG**, a **Joy-IT SBC-LCD84x48 REV2** 84×48 PCD8544 LCD, and an **Adafruit 3942 HC-SR04** distance sensor. A target at 10 cm or closer moves the paddle right. A target at 50 cm or farther moves it left. A 30 cm target centers it. The existing `nokia5110-invaders-lab` identifier remains stable for published links; the game is Breakout.

## Build and run

From the repository root:

```bash
cargo build -p nokia5110-invaders-lab --release --target thumbv7em-none-eabihf
cargo run -p labwired-cli -- test --script examples/nokia5110-invaders-lab/io-smoke.yaml
cargo test -p labwired-core --features event-scheduler --release \
  --test e2e_nokia5110_invaders -- --ignored --nocapture
```

The ELF is `target/thumbv7em-none-eabihf/release/nokia5110-invaders-lab`. Flash that ELF through the Nucleo's ST-LINK using STM32CubeProgrammer or an equivalent SWD programmer. Keep the normal ST-LINK jumpers and power the board through its USB Mini-B connector. The firmware keeps the 4 MHz MSI reset clock; it does not configure a PLL.

The 5 Mstep CLI smoke preserves the original clean-run baseline. The separate real-ELF regression suite checks displayed output, near/far paddle control, the 2 cm minimum, disconnected and stuck-high ECHO, batched execution, and walk-enabled versus walk-deleted framebuffer parity. An execution-only smoke does not prove those behaviors.

## Exact wiring

| Module contact | Nucleo contact | Signal |
| --- | --- | --- |
| LCD REV2 GND | CN5.7 or CN6.6 | GND |
| LCD REV2 LIGHT | GND | Active-low backlight |
| LCD REV2 VCC | CN6.4 | 3.3 V |
| LCD REV2 CLK | CN5.6, D13 | PA5 / SPI1 SCK |
| LCD REV2 DIN | CN5.4, D11 | PA7 / SPI1 MOSI |
| LCD REV2 DC | CN5.2, D9 | PC7 |
| LCD REV2 CE | CN5.3, D10 | PB6 |
| LCD REV2 RST | CN5.1, D8 | PA9 |
| HC-SR04 VCC | CN6.5 | 5 V |
| HC-SR04 TRIG | CN9.8, D7 | PA8 |
| HC-SR04 ECHO | 10 kΩ series, then CN9.7, D6 | PB10 |
| Divider lower resistor | PB10 to GND | 18 kΩ |
| HC-SR04 GND | GND | Common ground |

Use 1% divider resistors. The nominal divider output is 3.214 V for 5 V ECHO. At a 5.25 V input and opposite 1% resistor tolerances, its upper value is 3.399 V. The external 18 kΩ resistor also holds a disconnected input low; the firmware does not enable an internal pull that would change the divider ratio. Inspect the actual module revision and pin labels before connecting it. The REV2 manual lists labels rather than pad numbers, and its square pad is RST. Joy-IT REV1 uses a different pinout and backlight connection.

## Timing contract

TIM2 counts at 1 MHz from the 4 MHz APB1 reset clock with prescaler 3. Firmware measures ECHO in microseconds rather than calibrated instruction-loop iterations. It produces a 12 µs trigger, waits up to 30 ms for either ECHO edge, accepts pulse widths from 80 to 25,000 µs, and starts ranging no more often than once per 70 ms. Invalid, absent, or stuck ECHO returns the paddle toward center and clears its measurement history. Valid measurements use a three-sample median and a one-quarter moving average.

The system manifest declares the same **4 MHz** clock for the CPU and the existing HC-SR04 model. It caps the peripheral grid at **32 cycles = 8 µs** with the generic `max_peripheral_tick_interval` field. Both native and custom-runner advance APIs enforce this cap even if a host requests a larger interval. The current recommended batch request is 1024 cycles, or 256 µs at 4 MHz; an unrestricted grid can miss the nominal 116 µs pulse at 2 cm.

The effective grid corresponds to approximately **0.138 cm per 8 µs** using the sensor's 58 µs/cm nominal conversion. This is timing quantization, not a physical accuracy claim. GPIO polling latency, MSI tolerance, acoustic conditions, sensor tolerance, and target shape remain additional errors. The selected sensor's specified range is 2–400 cm; the game maps the practical 10–50 cm hand-control span to the paddle and clamps valid distances outside that span to the appropriate edge.

`walk_deleted: true` retains the existing event scheduler path. TIM2 reads derive elapsed cycles from the generic timer model. No per-device simulator or hidden clock calibration is added.

## Physical qualification

Model regressions verify digital behavior. They do not establish PCB assembly quality, real sensor accuracy, LCD contrast, or enclosure fit. A physical build must measure the 3.3 V and 5 V rails, the divided ECHO high level, trigger spacing, and the 1 MHz SPI clock, then compare the observed game response with the supplied model vectors. Resolve the framebuffer symbol from the exact linked ELF before reading it through SWD; its address is not fixed across builds.

Primary hardware references:

- [ST UM1724, Nucleo-64 MB1136 user manual](https://www.st.com/resource/en/user_manual/dm00105823-stm32-nucleo-64-boards-mb1136-stmicroelectronics.pdf)
- [ST STM32L476 datasheet](https://www.st.com/resource/en/datasheet/stm32l476rg.pdf)
- [Joy-IT SBC-LCD84x48 manual, 2024-03-18](https://www.joy-it.net/files/files/Produkte/SBC-LCD84x48/SBC-LCD84x48_Manual_2024-03-18.pdf)
- [Adafruit 3942 HC-SR04 specifications](https://www.adafruit.com/product/3942)
