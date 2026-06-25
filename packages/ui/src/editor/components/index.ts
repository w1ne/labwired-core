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
import { max31855Component } from './max31855';
import { mpu6050Component } from './mpu6050';
import { neo6mGpsComponent } from './neo6m-gps';
import { bg770aCellularComponent } from './bg770a-cellular';
import { ntcThermistorComponent } from './ntc-thermistor';
import { mlx90640Component } from './mlx90640';
// Displays
import { oledSsd1306Component } from './oled-ssd1306';
import { pcd8544Component } from './pcd8544';
import { ledMatrixComponent } from './led-matrix';
import { ili9341TftComponent } from './ili9341-tft';
import { epdSsd1680TricolorComponent } from './epd-ssd1680-tricolor';
import { epdUc8151dTricolorComponent } from './epd-uc8151d-tricolor';
// Passives
import { capacitorComponent } from './capacitor';
import { diodeComponent } from './diode';
import { transistorComponent } from './transistor';
// ICs
import { shiftRegister74hc595Component } from './shift-register-74hc595';
import { sn74hc165Component } from './sn74hc165';
import { iolinkMasterComponent } from './iolink-master';
import { m12IoLinkComponent } from './m12-iolink';
import { iolinkTransceiverComponent } from './iolink-transceiver';
import { canTransceiverComponent } from './can-transceiver';
import { canDiagnosticToolComponent } from './can-diagnostic-tool';
import { motorDriverL293dComponent } from './motor-driver-l293d';
// Tools
import { logicAnalyzerComponent } from './logic-analyzer';
import { noteComponent } from './note';
import { textAnnotationComponent } from './text-annotation';
// Board MCUs
import { arduinoUnoComponent } from './boards/arduino-uno';
import { esp32Component } from './boards/esp32';
import { esp32C3SuperMiniComponent } from './boards/esp32-c3-supermini';
import { esp32S3ZeroComponent } from './boards/esp32-s3-zero';
import { rpiPicoComponent } from './boards/rpi-pico';
import { nrf52840DkComponent } from './boards/nrf52840-dk';
import { stm32DevComponent } from './boards/stm32-dev';
import { nucleoH563ziComponent } from './boards/nucleo-h563zi';
import { nucleoF401reComponent } from './boards/nucleo-f401re';
import { nucleoL476rgComponent } from './boards/nucleo-l476rg';
import { stm32BlackpillComponent } from './boards/stm32-blackpill';

/** All available component definitions, keyed by type. */
export const COMPONENT_REGISTRY: Map<string, ComponentDef> = new Map([
  // MCUs
  [mcuComponent.type, mcuComponent],
  [arduinoUnoComponent.type, arduinoUnoComponent],
  [stm32DevComponent.type, stm32DevComponent],
  [nucleoH563ziComponent.type, nucleoH563ziComponent],
  [nucleoF401reComponent.type, nucleoF401reComponent],
  [nucleoL476rgComponent.type, nucleoL476rgComponent],
  [stm32BlackpillComponent.type, stm32BlackpillComponent],
  [esp32Component.type, esp32Component],
  [esp32C3SuperMiniComponent.type, esp32C3SuperMiniComponent],
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
  [max31855Component.type, max31855Component],
  [mpu6050Component.type, mpu6050Component],
  [neo6mGpsComponent.type, neo6mGpsComponent],
  [bg770aCellularComponent.type, bg770aCellularComponent],
  [ntcThermistorComponent.type, ntcThermistorComponent],
  [mlx90640Component.type, mlx90640Component],
  // Displays
  [sevenSegmentComponent.type, sevenSegmentComponent],
  [lcd1602Component.type, lcd1602Component],
  [oledSsd1306Component.type, oledSsd1306Component],
  [pcd8544Component.type, pcd8544Component],
  [ledMatrixComponent.type, ledMatrixComponent],
  [ili9341TftComponent.type, ili9341TftComponent],
  [epdSsd1680TricolorComponent.type, epdSsd1680TricolorComponent],
  [epdUc8151dTricolorComponent.type, epdUc8151dTricolorComponent],
  // Passives
  [resistorComponent.type, resistorComponent],
  [capacitorComponent.type, capacitorComponent],
  [diodeComponent.type, diodeComponent],
  [transistorComponent.type, transistorComponent],
  // ICs
  [shiftRegister74hc595Component.type, shiftRegister74hc595Component],
  [sn74hc165Component.type, sn74hc165Component],
  [iolinkMasterComponent.type, iolinkMasterComponent],
  [m12IoLinkComponent.type, m12IoLinkComponent],
  [iolinkTransceiverComponent.type, iolinkTransceiverComponent],
  [canTransceiverComponent.type, canTransceiverComponent],
  [canDiagnosticToolComponent.type, canDiagnosticToolComponent],
  [motorDriverL293dComponent.type, motorDriverL293dComponent],
  // Tools
  [logicAnalyzerComponent.type, logicAnalyzerComponent],
  [noteComponent.type, noteComponent],
  [textAnnotationComponent.type, textAnnotationComponent],
]);

import { defineComponent, genericComponentDef, makeGenericRender } from './generic';
export { defineComponent, genericComponentDef, makeGenericRender, renderComponentBody } from './generic';

/** Register (or override) a component at runtime — lets a consumer (e.g. proto.cat)
 *  inject its catalog parts as data-driven defs so the board can render anything. */
export function registerComponentDef(def: ComponentDef): void {
  COMPONENT_REGISTRY.set(def.type, def);
}

/** Register many parts from data in one call. */
export function registerComponents(
  parts: Array<Partial<ComponentDef> & { type: string; label: string }>,
): void {
  for (const p of parts) COMPONENT_REGISTRY.set(p.type, defineComponent(p));
}

/** Always return a usable def: the registered one, or a synthesized generic box
 *  for an unregistered type, with a render guaranteed (generic when absent). So no
 *  part ever fails to draw. */
export function resolveComponentDef(type: string): ComponentDef {
  const def = COMPONENT_REGISTRY.get(type) ?? genericComponentDef(type);
  if (!def.render) def.render = makeGenericRender(def);
  return def;
}

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
