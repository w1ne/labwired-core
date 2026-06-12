/**
 * Bundled-diagram regression sweep (Fix 1, second part).
 *
 * CONTRACT: for every seeded/bundled diagram from the playground,
 * composeDiagnostics() must yield NO more error-severity codes than the legacy
 * diagnoseDiagram() alone yields.
 *
 * In other words, the kernel ERC must not introduce NEW blocking errors on
 * diagrams that the legacy system already accepted (or rejected for the same
 * reason).
 *
 * This test was designed to FAIL before Fix 1 (power-pin etypes for all boards)
 * because mcu:VCC / mcu:GND / mcu:VDD on STM32/nRF boards defaulted to
 * bidirectional, causing spurious PWR_RAIL_UNDRIVEN errors in the kernel ERC
 * that were NOT present in the legacy diagnostics.
 */

import { describe, expect, it } from 'vitest';
import { composeDiagnostics, diagnoseDiagram } from '../src';
import type { ValidateDiagram } from '../src';

// ---------------------------------------------------------------------------
// Inline diagram literals extracted from packages/playground/src/App.tsx.
// Only diagrams with non-trivial wiring are included — bare MCU-only diagrams
// produce no interesting ERC findings.
// ---------------------------------------------------------------------------

const stm32f103McuPart = { id: 'mcu', type: 'stm32-dev' };
const stm32l476McuPart = { id: 'mcu', type: 'mcu' }; // nucleo-l476rg maps to generic mcu in legacy

const BUNDLED_DIAGRAMS: Array<{ name: string; diagram: ValidateDiagram }> = [
  {
    name: 'stm32f103-blinky',
    diagram: {
      board: 'stm32f103',
      parts: [
        stm32f103McuPart,
        { id: 'led_pa5', type: 'led' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led_pa5', pin: 'A' } },
      ],
    },
  },
  {
    name: 'ssd1306-hello-lab (stm32f103 + VCC/GND)',
    diagram: {
      board: 'stm32f103',
      parts: [
        stm32f103McuPart,
        { id: 'oled', type: 'oled-ssd1306' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'oled', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'oled', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'oled', pin: 'SCL' } },
        { from: { part: 'mcu', pin: 'PB7' }, to: { part: 'oled', pin: 'SDA' } },
      ],
    },
  },
  {
    name: 'bme280-weather-lab (stm32f103 + VCC/GND)',
    diagram: {
      board: 'stm32f103',
      parts: [
        stm32f103McuPart,
        { id: 'bme280', type: 'bme280' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'bme280', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'bme280', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'bme280', pin: 'SCL' } },
        { from: { part: 'mcu', pin: 'PB7' }, to: { part: 'bme280', pin: 'SDA' } },
      ],
    },
  },
  {
    name: 'mpu6050-sensor-lab (stm32f103 + VCC/GND)',
    diagram: {
      board: 'stm32f103',
      parts: [
        stm32f103McuPart,
        { id: 'mpu6050', type: 'mpu6050' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'mpu6050', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'mpu6050', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'mpu6050', pin: 'SCL' } },
        { from: { part: 'mcu', pin: 'PB7' }, to: { part: 'mpu6050', pin: 'SDA' } },
      ],
    },
  },
  {
    name: 'adxl345-sensor-lab (stm32f103 + VCC/GND)',
    diagram: {
      board: 'stm32f103',
      parts: [
        stm32f103McuPart,
        { id: 'adxl345', type: 'adxl345' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'adxl345', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'adxl345', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'adxl345', pin: 'SCL' } },
        { from: { part: 'mcu', pin: 'PB7' }, to: { part: 'adxl345', pin: 'SDA' } },
      ],
    },
  },
  {
    name: 'max31855-thermocouple-lab (stm32f103 + VCC/GND)',
    diagram: {
      board: 'stm32f103',
      parts: [
        stm32f103McuPart,
        { id: 'tc1', type: 'max31855' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'tc1', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'tc1', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'PA4' }, to: { part: 'tc1', pin: 'CS' } },
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'tc1', pin: 'SCK' } },
        { from: { part: 'mcu', pin: 'PA6' }, to: { part: 'tc1', pin: 'DO' } },
      ],
    },
  },
  {
    name: 'neo6m-gps-lab (stm32f103 + VCC/GND)',
    diagram: {
      board: 'stm32f103',
      parts: [
        stm32f103McuPart,
        { id: 'gps', type: 'neo6m-gps' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' },  to: { part: 'gps', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' },  to: { part: 'gps', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'PA9' },  to: { part: 'gps', pin: 'RX' } },
        { from: { part: 'mcu', pin: 'PA10' }, to: { part: 'gps', pin: 'TX' } },
      ],
    },
  },
  {
    name: 'ili9341-tft-lab (stm32f103 + VCC/GND)',
    diagram: {
      board: 'stm32f103',
      parts: [
        stm32f103McuPart,
        { id: 'tft', type: 'ili9341' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'tft', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'tft', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'PA4' }, to: { part: 'tft', pin: 'CS' } },
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'tft', pin: 'SCK' } },
        { from: { part: 'mcu', pin: 'PA7' }, to: { part: 'tft', pin: 'MOSI' } },
        { from: { part: 'mcu', pin: 'PB0' }, to: { part: 'tft', pin: 'DC' } },
        { from: { part: 'mcu', pin: 'PB1' }, to: { part: 'tft', pin: 'RESET' } },
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'tft', pin: 'LED' } },
      ],
    },
  },
  {
    name: 'epaper-tricolor-lab (stm32f103 + VCC/GND + PC7)',
    diagram: {
      board: 'stm32f103',
      parts: [
        stm32f103McuPart,
        { id: 'epaper', type: 'ssd1680_tricolor_290' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'epaper', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'epaper', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'PA7' }, to: { part: 'epaper', pin: 'DIN' } },
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'epaper', pin: 'CLK' } },
        { from: { part: 'mcu', pin: 'PA4' }, to: { part: 'epaper', pin: 'CS' } },
        { from: { part: 'mcu', pin: 'PB0' }, to: { part: 'epaper', pin: 'DC' } },
        { from: { part: 'mcu', pin: 'PA9' }, to: { part: 'epaper', pin: 'RST' } },
        { from: { part: 'mcu', pin: 'PC7' }, to: { part: 'epaper', pin: 'BUSY' } },
      ],
    },
  },
  {
    name: 'nokia5110-invaders-lab (stm32l476 + VCC/GND)',
    diagram: {
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'mcu' },
        { id: 'lcd', type: 'pcd8544' },
        { id: 'dist', type: 'ultrasonic' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'lcd', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'lcd', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'lcd', pin: 'CLK' } },
        { from: { part: 'mcu', pin: 'PA7' }, to: { part: 'lcd', pin: 'DIN' } },
        { from: { part: 'mcu', pin: 'PC7' }, to: { part: 'lcd', pin: 'DC' } },
        { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'lcd', pin: 'CE' } },
        { from: { part: 'mcu', pin: 'PA9' }, to: { part: 'lcd', pin: 'RST' } },
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'dist', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'dist', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'PA8' }, to: { part: 'dist', pin: 'TRIG' } },
        { from: { part: 'mcu', pin: 'PB10' }, to: { part: 'dist', pin: 'ECHO' } },
      ],
    },
  },
  {
    name: 'nrf52840-proximity-lab (nrf52840 + VDD/GND)',
    diagram: {
      board: 'nrf52840',
      parts: [
        { id: 'mcu', type: 'nrf52840-dk' },
        { id: 'ultrasonic', type: 'ultrasonic' },
        { id: 'alarm_led', type: 'led' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VDD' }, to: { part: 'ultrasonic', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'ultrasonic', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'P0.04' }, to: { part: 'ultrasonic', pin: 'TRIG' } },
        { from: { part: 'mcu', pin: 'P0.05' }, to: { part: 'ultrasonic', pin: 'ECHO' } },
        { from: { part: 'mcu', pin: 'P0.06' }, to: { part: 'alarm_led', pin: 'A' } },
      ],
    },
  },
  {
    name: 'esp32-epaper-lab (esp32 + 3V3/GND)',
    diagram: {
      board: 'esp32',
      parts: [
        { id: 'mcu', type: 'esp32' },
        { id: 'epaper', type: 'ssd1680_tricolor_290' },
      ],
      wires: [
        { from: { part: 'mcu', pin: '3V3' },    to: { part: 'epaper', pin: 'VCC' } },
        { from: { part: 'mcu', pin: 'GND' },    to: { part: 'epaper', pin: 'GND' } },
        { from: { part: 'mcu', pin: 'GPIO23' }, to: { part: 'epaper', pin: 'DIN' } },
        { from: { part: 'mcu', pin: 'GPIO18' }, to: { part: 'epaper', pin: 'CLK' } },
        { from: { part: 'mcu', pin: 'GPIO5' },  to: { part: 'epaper', pin: 'CS' } },
        { from: { part: 'mcu', pin: 'GPIO17' }, to: { part: 'epaper', pin: 'DC' } },
        { from: { part: 'mcu', pin: 'GPIO16' }, to: { part: 'epaper', pin: 'RST' } },
        { from: { part: 'mcu', pin: 'GPIO4' },  to: { part: 'epaper', pin: 'BUSY' } },
      ],
    },
  },
  {
    name: 'ntc-thermistor-lab (stm32f103, ADC only, no power wires)',
    diagram: {
      board: 'stm32f103',
      parts: [
        stm32f103McuPart,
        { id: 'ntc', type: 'ntc-thermistor' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PA0' }, to: { part: 'ntc', pin: 'A' } },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'ntc', pin: 'B' } },
      ],
    },
  },
];

describe('bundled diagram regression sweep', () => {
  it.each(BUNDLED_DIAGRAMS)(
    '$name: composeDiagnostics() adds no new error codes vs legacyDiagnose()',
    ({ diagram }) => {
      const legacyResult = diagnoseDiagram(diagram);
      const legacyErrorCodes = new Set<string>(
        legacyResult.filter((d) => d.severity === 'error').map((d) => d.code as string),
      );

      const composedResult = composeDiagnostics(diagram);
      const composedErrorCodes = new Set(
        composedResult.diagnostics.filter((d) => d.severity === 'error').map((d) => d.code),
      );

      // Find error codes present in composed but NOT in legacy — these are
      // regressions introduced by the kernel ERC.
      const newErrorCodes = [...composedErrorCodes].filter((c) => !legacyErrorCodes.has(c));

      expect(newErrorCodes, [
        `composeDiagnostics introduced new error codes not in legacyDiagnose for "${diagram.board}" diagram.`,
        `Legacy errors: [${[...legacyErrorCodes].join(', ')}]`,
        `Composed errors: [${[...composedErrorCodes].join(', ')}]`,
        `New errors: [${newErrorCodes.join(', ')}]`,
      ].join('\n')).toHaveLength(0);
    },
  );
});
