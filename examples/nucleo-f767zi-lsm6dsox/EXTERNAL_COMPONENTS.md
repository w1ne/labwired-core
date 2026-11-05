# External components

- **ST LSM6DSOXTR**, 6-axis inertial sensor, soldered to a customer carrier or
  breakout. This example exercises only its I2C accelerometer path. The chip's
  analog and I/O supplies must meet the ST datasheet; the example uses a 3.3 V
  board design.
- **NUCLEO-F767ZI**, existing host board. I2C1 SCL/SDA use PB8/PB9 (D15/D14);
  USART3 PD8/PD9 is the ST-LINK virtual COM port; LD1 is PB0.

The sensor itself needs CS high for I2C and SDO/SA0 low for address `0x6A`.
On a customer carrier, route VDD, VDDIO, common ground, I2C pull-ups, and the
sensor's required local decoupling according to ST's [datasheet](https://www.st.com/resource/en/datasheet/lsm6dsox.pdf).
