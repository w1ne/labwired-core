# Required source documents

- ST LSM6DSOX datasheet DS12814 Rev. 4:
  <https://www.st.com/resource/en/datasheet/lsm6dsox.pdf>
  - WHO_AM_I, register address `0x0F`, value `0x6C` (§9.14).
  - CTRL1_XL ODR/full-scale selection and 416 Hz setting (§9.15).
  - Accelerometer output registers `0x28`–`0x2D` (§§9.34–9.36).
  - ±2 g sensitivity `0.061 mg/LSB` (Table 2).
- ST NUCLEO-144 board manual UM1974:
  <https://www.st.com/resource/en/user_manual/dm00244518-stm32-nucleo144-boards-mb1137-stmicroelectronics.pdf>
  - Arduino D15/D14 are PB8/PB9 for I2C1 on NUCLEO-F767ZI.
- ST product lifecycle page:
  <https://www.st.com/en/mems-and-sensors/lsm6dsox.html>
- Current distributor availability is recorded in
  [`evidence/vendor-status.md`](evidence/vendor-status.md).
