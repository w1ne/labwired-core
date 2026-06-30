### Project Setup

To program the Raspberry Pi Pico, we use the Raspberry Pi Pico VS Code extension. 

Follow these instructions to flash the example:

1. Open the Raspberry Pi Pico VS Code extension and create a new **C/C++ project**.
2. Select your board type.
3. Enable the **I2C interface** feature.
4. Enable **Console over USB**.
5. Click **Create**.
6. Copy all driver .h and .c files into the new pico project.
7. Copy the example usage into the pico project
8. Replace the sensirion_i2c_hal.c file with the file from the Raspberry Pi Pico sample implementation
9. In your CMakeLists.txt, replace the add_executable block with:
    ```cmake
    add_executable(<project_name>
        scd4x_i2c_example_usage.c
        sensirion_i2c.c
        sensirion_i2c_hal.c
        scd4x_i2c.c
        sensirion_common.c
    )
    ```
10. Replace <project_name> with your project name
11. Connect your sensor and flash the firmware.

Once flashed successfully, the Raspberry Pi Pico will print sensor readings over the USB serial console.

### Connecting the Sensor

- SDA: GPIO4 (Pin 6)
- SCL: GPIO5 (Pin 7)