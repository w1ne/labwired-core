/**
 * @deprecated Use compile() from './compile' instead.
 *
 * Legacy diagram-to-config converter. Now delegates to shared emitters
 * extracted into ./compile/emitters.ts so output is string-identical to
 * what compile() produces for v1/wire-based diagrams.
 */
import type { Diagram } from './types';
import { CHIP_YAMLS } from './chip-yamls';
import {
  I2C_DEVICE_ADDRESSES,
  SPI_DEVICE_TYPES,
  emitUltrasonic,
  emitPcd8544,
  emitSn74hc165,
  emitIolinkMaster,
  emitLegacyI2cDevice,
  emitSpiDevice,
  emitNeo6mGps,
  emitBoardIoFromWires,
  buildSystemYaml,
  i2cPeripheralForPartWire,
} from './compile/emitters';

/**
 * Convert a visual diagram into system YAML + chip YAML for the WASM simulator.
 * If chipYamlOverride is provided, it is used instead of the built-in CHIP_YAMLS lookup.
 */
export function diagramToConfig(
  diagram: Diagram,
  chipYamlOverride?: string,
): { systemYaml: string; chipYaml: string } {
  const chipYaml = chipYamlOverride ?? CHIP_YAMLS[diagram.board];
  if (!chipYaml) {
    throw new Error(`Unknown board: ${diagram.board}. Provide a chipYamlOverride or add it to CHIP_YAMLS.`);
  }

  const boardIoEntries: string[] = [];
  const externalDeviceEntries: string[] = [];

  // Ultrasonic (HC-SR04)
  for (const part of diagram.parts) {
    if (part.type !== 'ultrasonic') continue;
    const { externalDevice } = emitUltrasonic(diagram, part.id);
    if (externalDevice) externalDeviceEntries.push(externalDevice);
  }

  // PCD8544
  for (const part of diagram.parts) {
    if (part.type !== 'pcd8544') continue;
    const { externalDevice, boardIo } = emitPcd8544(diagram, part.id);
    if (externalDevice) externalDeviceEntries.push(externalDevice);
    if (boardIo) boardIoEntries.push(boardIo);
  }

  // SN74HC165
  for (const part of diagram.parts) {
    if (part.type !== 'sn74hc165') continue;
    const { externalDevice } = emitSn74hc165(diagram, part.id);
    if (externalDevice) externalDeviceEntries.push(externalDevice);
  }

  // IO-Link master
  for (const part of diagram.parts) {
    if (part.type !== 'iolink-master') continue;
    const { externalDevice } = emitIolinkMaster(diagram, part.id);
    if (externalDevice) externalDeviceEntries.push(externalDevice);
  }

  // Legacy I2C devices (adxl345, mpu6050, bme280, oled-ssd1306)
  for (const part of diagram.parts) {
    const address = I2C_DEVICE_ADDRESSES[part.type];
    if (address === undefined) continue;
    const connection = i2cPeripheralForPartWire(diagram, part.id);
    if (!connection) continue;
    const { externalDevice, boardIo } = emitLegacyI2cDevice(part.id, part.type, connection, address);
    externalDeviceEntries.push(externalDevice);
    boardIoEntries.push(boardIo);
  }

  // SPI devices (ili9341, max31855, ssd1680_tricolor_290)
  for (const part of diagram.parts) {
    if (!SPI_DEVICE_TYPES.has(part.type)) continue;
    const { externalDevice, boardIo } = emitSpiDevice(diagram, part.id, part.type);
    if (externalDevice) externalDeviceEntries.push(externalDevice);
    if (boardIo) boardIoEntries.push(boardIo);
  }

  // NEO-6M GPS
  for (const part of diagram.parts) {
    if (part.type !== 'neo6m-gps') continue;
    const { externalDevice, boardIo } = emitNeo6mGps(diagram, part.id);
    if (externalDevice) externalDeviceEntries.push(externalDevice);
    if (boardIo) boardIoEntries.push(boardIo);
  }

  // Wire-based board_io for all remaining part types
  boardIoEntries.push(...emitBoardIoFromWires(diagram));

  const systemYaml = buildSystemYaml(externalDeviceEntries, boardIoEntries);
  return { systemYaml, chipYaml };
}
