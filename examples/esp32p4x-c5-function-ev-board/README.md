# ESP32-P4X-C5-Function-EV-Board + BME280

**Demand rank:** ESP32 is #1 in the measured MCU list (38%). The survey does
not score ESP32-P4 separately; this adds a tested peripheral path to one board
in the highest-demand family. Vendor status is recorded in
[`vendor-status.md`](vendor-status.md).

## Connect the sensor

Use an Adafruit BME280 STEMMA QT breakout (or a compatible BME280 I2C module).
The P4 board guide maps these J1 pins:

| ESP32-P4 board | BME280 breakout |
| --- | --- |
| J1 pin 1, 3V3 | VIN / 3V |
| J1 pin 6, GND | GND |
| J1 pin 3, GPIO7 | SDA |
| J1 pin 5, GPIO8 | SCL |

The system descriptor uses I2C address `0x77`. Set a breakout's address jumper
to `0x77` if it defaults to `0x76`.

## Copy-paste example

[`firmware/src/main.rs`](firmware/src/main.rs) is a complete bare-metal Rust
example for configuring the GPIO matrix, initializing HP I2C0, and reading the
BME280 chip ID and raw temperature registers. This excerpt goes in `main`
after the GPIO/I2C setup calls in the full source file:

```rust
unsafe {
    let mut chip_id = [0u8; 1];
    if !read_bme280_registers(0xD0, &mut chip_id) || chip_id[0] != 0x60 {
        uart_str(b"P4_BME280_FAIL CHIP_ID\n");
        wait_forever();
    }

    let mut temperature = [0u8; 3];
    if !read_bme280_registers(0xFA, &mut temperature) {
        uart_str(b"P4_BME280_FAIL TEMP_READ\n");
        wait_forever();
    }
    let raw_temperature = (u32::from(temperature[0]) << 12)
        | (u32::from(temperature[1]) << 4)
        | (u32::from(temperature[2]) >> 4);
    if raw_temperature != 0x082DC0 {
        uart_str(b"P4_BME280_FAIL TEMP\n");
        wait_forever();
    }
}
```

Run the same firmware build and simulator smoke used for the committed evidence:

```bash
examples/esp32p4x-c5-function-ev-board/ci/build.sh
examples/esp32p4x-c5-function-ev-board/ci/test.sh
```

The smoke prints `P4_BME280_ID=0x60_RAW_TEMP=0x082DC0_GPIO7_GPIO8` and
`P4X_C5_FUNCTION_EV_OK` on UART0 after checking both values.

## Modeled scope

The simulator starts a bare ELF on the P4 HP RISC-V core. This profile models
UART0 output, the GPIO data/enable/input head, the GPIO7/8 I2C matrix selector
registers, HP I2C0 command-list behavior, and a BME280 slave. The IO_MUX block
stores the two pad setup registers; electrical pad behavior is not simulated.
The smoke proves the internal UART console and a BME280 register transaction,
not the physical header or USB output.

Reset ROM and the second HP core are not run. The C5 radio module, LP core,
SPI flash boot, PSRAM, display, touch controller, camera, audio, Ethernet, USB,
MicroSD, other I2C routes, and other GPIO matrix signals remain outside this
profile.
