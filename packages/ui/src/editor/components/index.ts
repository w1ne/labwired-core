import type { ComponentDef } from '../types';
import { mcuComponent } from './mcu';
import { ledComponent } from './led';
import { buttonComponent } from './button';
import { resistorComponent } from './resistor';
import { potentiometerComponent } from './potentiometer';
// Output
import { rgbLedComponent } from './rgb-led';
import { sevenSegmentComponent } from './seven-segment';
import { lcd1602Component } from './lcd1602';
import { buzzerComponent } from './buzzer';
import { servoComponent } from './servo';
import { neopixelComponent } from './neopixel';
// Input
import { slideSwitchComponent } from './slide-switch';
import { dipSwitchComponent } from './dip-switch';
import { rotaryEncoderComponent } from './rotary-encoder';
import { keypadComponent } from './keypad';
// Sensors
import { dht22Component } from './dht22';
import { pirSensorComponent } from './pir-sensor';
import { ultrasonicComponent } from './ultrasonic';
import { ldrComponent } from './ldr';
import { adxl345Component } from './adxl345';
import { bme280Component } from './bme280';
import { mpu6050Component } from './mpu6050';
// Displays
import { oledSsd1306Component } from './oled-ssd1306';
import { ledMatrixComponent } from './led-matrix';
// Passives
import { capacitorComponent } from './capacitor';
import { diodeComponent } from './diode';
import { transistorComponent } from './transistor';
// ICs
import { shiftRegister74hc595Component } from './shift-register-74hc595';
import { motorDriverL293dComponent } from './motor-driver-l293d';
// Board MCUs
import { arduinoUnoComponent } from './boards/arduino-uno';
import { esp32Component } from './boards/esp32';
import { esp32S3ZeroComponent } from './boards/esp32-s3-zero';
import { rpiPicoComponent } from './boards/rpi-pico';
import { nrf52840DkComponent } from './boards/nrf52840-dk';
import { stm32DevComponent } from './boards/stm32-dev';

/** All available component definitions, keyed by type. */
export const COMPONENT_REGISTRY: Map<string, ComponentDef> = new Map([
  // MCUs
  [mcuComponent.type, mcuComponent],
  [arduinoUnoComponent.type, arduinoUnoComponent],
  [stm32DevComponent.type, stm32DevComponent],
  [esp32Component.type, esp32Component],
  [esp32S3ZeroComponent.type, esp32S3ZeroComponent],
  [rpiPicoComponent.type, rpiPicoComponent],
  [nrf52840DkComponent.type, nrf52840DkComponent],
  // Output
  [ledComponent.type, ledComponent],
  [rgbLedComponent.type, rgbLedComponent],
  [buzzerComponent.type, buzzerComponent],
  [servoComponent.type, servoComponent],
  [neopixelComponent.type, neopixelComponent],
  // Input
  [buttonComponent.type, buttonComponent],
  [potentiometerComponent.type, potentiometerComponent],
  [slideSwitchComponent.type, slideSwitchComponent],
  [dipSwitchComponent.type, dipSwitchComponent],
  [rotaryEncoderComponent.type, rotaryEncoderComponent],
  [keypadComponent.type, keypadComponent],
  // Sensors
  [dht22Component.type, dht22Component],
  [pirSensorComponent.type, pirSensorComponent],
  [ultrasonicComponent.type, ultrasonicComponent],
  [ldrComponent.type, ldrComponent],
  [adxl345Component.type, adxl345Component],
  [bme280Component.type, bme280Component],
  [mpu6050Component.type, mpu6050Component],
  // Displays
  [sevenSegmentComponent.type, sevenSegmentComponent],
  [lcd1602Component.type, lcd1602Component],
  [oledSsd1306Component.type, oledSsd1306Component],
  [ledMatrixComponent.type, ledMatrixComponent],
  // Passives
  [resistorComponent.type, resistorComponent],
  [capacitorComponent.type, capacitorComponent],
  [diodeComponent.type, diodeComponent],
  [transistorComponent.type, transistorComponent],
  // ICs
  [shiftRegister74hc595Component.type, shiftRegister74hc595Component],
  [motorDriverL293dComponent.type, motorDriverL293dComponent],
]);

/** Component definitions grouped by category (excludes MCU). */
export function getComponentsByCategory(): Record<string, ComponentDef[]> {
  const groups: Record<string, ComponentDef[]> = {};
  for (const def of COMPONENT_REGISTRY.values()) {
    if (def.category === 'mcu') continue;
    const cat = def.category;
    if (!groups[cat]) groups[cat] = [];
    groups[cat].push(def);
  }
  return groups;
}
