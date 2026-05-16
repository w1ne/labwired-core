# MPU6050 Sensor Lab

STM32F103 + MPU6050 6-DoF IMU over simulated I²C.

## What it does

1. Wakes the MPU6050 by clearing the SLEEP bit in PWR_MGMT_1 (register 0x6B).
2. Reads WHO_AM_I (register 0x75) and prints it to UART — should be `0x68`.
3. Loops reading accelerometer (registers 0x3B–0x40) and gyroscope (registers 0x43–0x48) data,
   printing `AX= AY= AZ= GX= GY= GZ=` lines to UART1.

## Building

```bash
cargo build -p mpu6050-sensor-lab --release --target thumbv7m-none-eabi
```

## Running in LabWired playground

Select the **MPU6050 IMU** lab from the gallery or chip-row.
The Inspector card shows live accelerometer and gyroscope values.
