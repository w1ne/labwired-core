import { describe, expect, it } from 'vitest';
import { diagramToConfig } from './diagramToConfig';
import type { Diagram } from './types';

describe('diagramToConfig', () => {
  it('maps wired LED parts into board_io bindings', () => {
    const diagram: Diagram = {
      version: 1,
      board: 'stm32f103',
      parts: [
        { id: 'mcu', type: 'stm32-dev', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'led_custom', type: 'led', x: 200, y: 100, rotate: 0, attrs: { color: 'green' } },
      ],
      wires: [
        {
          from: { part: 'mcu', pin: 'PA5' },
          to: { part: 'led_custom', pin: 'A' },
          color: '#27c93f',
        },
      ],
    };

    const { systemYaml, chipYaml } = diagramToConfig(diagram);

    expect(systemYaml).toContain('id: "led_custom"');
    expect(systemYaml).toContain('peripheral: "gpioa"');
    expect(systemYaml).toContain('pin: 5');
    expect(systemYaml).toContain('kind: "led"');
    expect(chipYaml).toContain('name: "stm32f103c8"');
  });

  it('maps wired ultrasonic parts into HC-SR04 external devices', () => {
    const diagram: Diagram = {
      version: 1,
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'nucleo-l476rg', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'range1', type: 'ultrasonic', x: 200, y: 100, rotate: 0, attrs: { distance: '42' } },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PA8' }, to: { part: 'range1', pin: 'TRIG' }, color: '#06D6A0' },
        { from: { part: 'mcu', pin: 'PB10' }, to: { part: 'range1', pin: 'ECHO' }, color: '#118AB2' },
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'range1', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'range1', pin: 'GND' }, color: '#888888' },
      ],
    };

    const { systemYaml } = diagramToConfig(diagram);

    expect(systemYaml).toContain('external_devices:');
    expect(systemYaml).toContain('id: "range1"');
    expect(systemYaml).toContain('type: "hc-sr04"');
    expect(systemYaml).toContain('trig_pin: "PA8"');
    expect(systemYaml).toContain('echo_pin: "PB10"');
    expect(systemYaml).toContain('distance_cm: 42');
    expect(systemYaml).not.toContain('kind: "button"');
  });

  it('emits native IO-Link master devices for IO-Link components', () => {
    const diagram: Diagram = {
      version: 1,
      board: 'stm32f103',
      parts: [
        { id: 'mcu', type: 'stm32-dev', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'iolink_master', type: 'iolink-master', x: 200, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        {
          from: { part: 'mcu', pin: 'PA2' },
          to: { part: 'iolink_master', pin: 'TX' },
          color: '#3f8cff',
        },
      ],
    };

    const { systemYaml } = diagramToConfig(diagram);

    expect(systemYaml).toContain('id: "iolink_master"');
    expect(systemYaml).toContain('type: "iolink-master"');
    expect(systemYaml).toContain('connection: "uart2"');
    expect(systemYaml).toContain('pd_in_len: 1');
    expect(systemYaml).toContain('com: "COM2"');
  });

  it('emits CAN diagnostic tester devices from wired H563 CAN blocks', () => {
    const diagram: Diagram = {
      version: 1,
      board: 'stm32h563',
      parts: [
        { id: 'mcu', type: 'nucleo-h563zi', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'can_xcvr', type: 'can-transceiver', x: 300, y: 100, rotate: 0, attrs: {} },
        { id: 'uds_tester', type: 'can-diagnostic-tool', x: 500, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PD1' }, to: { part: 'can_xcvr', pin: 'TXD' }, color: '#06D6A0' },
        { from: { part: 'mcu', pin: 'PD0' }, to: { part: 'can_xcvr', pin: 'RXD' }, color: '#118AB2' },
        { from: { part: 'can_xcvr', pin: 'CAN_H' }, to: { part: 'uds_tester', pin: 'CAN_H' }, color: '#06D6A0' },
        { from: { part: 'can_xcvr', pin: 'CAN_L' }, to: { part: 'uds_tester', pin: 'CAN_L' }, color: '#118AB2' },
      ],
    };

    const { systemYaml } = diagramToConfig(diagram, 'name: "stm32h563-test"\n');

    expect(systemYaml).toContain('id: "uds_tester"');
    expect(systemYaml).toContain('type: "can-diagnostic-tester"');
    expect(systemYaml).toContain('connection: "fdcan1"');
  });

  it('maps the Nokia 5110 lab diagram into reusable external device contracts', () => {
    const diagram: Diagram = {
      version: 1,
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'nucleo-l476rg', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'lcd', type: 'pcd8544', x: 500, y: 60, rotate: 0, attrs: {} },
        { id: 'dist', type: 'ultrasonic', x: 500, y: 280, rotate: 0, attrs: { distance: '30' } },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'lcd', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'lcd', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'lcd', pin: 'CLK' }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PA7' }, to: { part: 'lcd', pin: 'DIN' }, color: '#B07BFF' },
        { from: { part: 'mcu', pin: 'PC7' }, to: { part: 'lcd', pin: 'DC' }, color: '#3DD68C' },
        { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'lcd', pin: 'CE' }, color: '#FFD166' },
        { from: { part: 'mcu', pin: 'PA9' }, to: { part: 'lcd', pin: 'RST' }, color: '#EF476F' },
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'dist', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'dist', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PA8' }, to: { part: 'dist', pin: 'TRIG' }, color: '#06D6A0' },
        { from: { part: 'mcu', pin: 'PB10' }, to: { part: 'dist', pin: 'ECHO' }, color: '#118AB2' },
      ],
    };

    const { systemYaml } = diagramToConfig(diagram);

    expect(systemYaml).toContain('id: "lcd"');
    expect(systemYaml).toContain('type: "pcd8544"');
    expect(systemYaml).toContain('connection: "spi1"');
    expect(systemYaml).toContain('cs_pin: "PB6"');
    expect(systemYaml).toContain('dc_pin: "PC7"');
    expect(systemYaml).toContain('id: "dist"');
    expect(systemYaml).toContain('type: "hc-sr04"');
    expect(systemYaml).toContain('trig_pin: "PA8"');
    expect(systemYaml).toContain('echo_pin: "PB10"');
    expect(systemYaml).toContain('distance_cm: 30');
    expect(systemYaml).toContain('cpu_hz: 250000');
  });

  it('maps the IO-Link lab diagram into native external devices', () => {
    const diagram: Diagram = {
      version: 1,
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'nucleo-l476rg', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'di_shifter', type: 'sn74hc165', x: 520, y: 70, rotate: 0, attrs: {} },
        { id: 'iolink_xcvr', type: 'iolink-transceiver', x: 500, y: 285, rotate: 0, attrs: {} },
        { id: 'iolink_master', type: 'iolink-master', x: 680, y: 285, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'di_shifter', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'di_shifter', pin: 'GND' }, color: '#888888' },
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'di_shifter', pin: 'CLK' }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PA6' }, to: { part: 'di_shifter', pin: 'QH' }, color: '#B07BFF' },
        { from: { part: 'mcu', pin: 'PA4' }, to: { part: 'di_shifter', pin: 'SH_LD' }, color: '#FFD166' },
        { from: { part: 'mcu', pin: 'PA2' }, to: { part: 'iolink_xcvr', pin: 'TXD' }, color: '#06D6A0' },
        { from: { part: 'mcu', pin: 'PA3' }, to: { part: 'iolink_xcvr', pin: 'RXD' }, color: '#118AB2' },
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'iolink_xcvr', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'iolink_xcvr', pin: 'GND' }, color: '#888888' },
        { from: { part: 'iolink_xcvr', pin: 'CQ' }, to: { part: 'iolink_master', pin: 'TX' }, color: '#F5B642' },
        { from: { part: 'iolink_xcvr', pin: 'CQ' }, to: { part: 'iolink_master', pin: 'RX' }, color: '#F5B642' },
        { from: { part: 'iolink_xcvr', pin: 'L+' }, to: { part: 'iolink_master', pin: 'L+' }, color: '#FF6B6B' },
      ],
    };

    const { systemYaml } = diagramToConfig(diagram);

    expect(systemYaml).toContain('id: "iolink_master"');
    expect(systemYaml).toContain('type: "iolink-master"');
    expect(systemYaml).toContain('connection: "uart2"');
    expect(systemYaml).toContain('pd_in_len: 1');
    expect(systemYaml).toContain('m_seq_type: 1');
    expect(systemYaml).toContain('com: "COM2"');
    expect(systemYaml).not.toContain('id: "iolink_xcvr"');
    expect(systemYaml).toContain('id: "di_shifter"');
    expect(systemYaml).toContain('type: "sn74hc165"');
    expect(systemYaml).toContain('connection: "spi1"');
    expect(systemYaml).toContain('cs_pin: "PA4"');
    expect(systemYaml).toContain('inputs: 165');
  });

  it('uses the 74HC165 input attribute as the native Rust device initial byte', () => {
    const diagram: Diagram = {
      version: 1,
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'nucleo-l476rg', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'di_shifter', type: 'sn74hc165', x: 520, y: 70, rotate: 0, attrs: { inputs: '170' } },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'di_shifter', pin: 'CLK' }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PA6' }, to: { part: 'di_shifter', pin: 'QH' }, color: '#B07BFF' },
        { from: { part: 'mcu', pin: 'PA4' }, to: { part: 'di_shifter', pin: 'SH_LD' }, color: '#FFD166' },
      ],
    };

    const { systemYaml } = diagramToConfig(diagram);

    expect(systemYaml).toContain('type: "sn74hc165"');
    expect(systemYaml).toContain('inputs: 170');
  });

  it('maps wired I2C sensors (BME280) into i2c external_devices + board_io', () => {
    const diagram: Diagram = {
      version: 1,
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'nucleo-l476rg', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'weather', type: 'bme280', x: 200, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PB6' }, to: { part: 'weather', pin: 'SCL' }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PB7' }, to: { part: 'weather', pin: 'SDA' }, color: '#B07BFF' },
        { from: { part: 'mcu', pin: 'VCC' }, to: { part: 'weather', pin: 'VCC' }, color: '#FF6B6B' },
        { from: { part: 'mcu', pin: 'GND' }, to: { part: 'weather', pin: 'GND' }, color: '#888888' },
      ],
    };

    const { systemYaml } = diagramToConfig(diagram);

    expect(systemYaml).toContain('id: "weather"');
    expect(systemYaml).toContain('type: "bme280"');
    expect(systemYaml).toContain('connection: "i2c1"');
    expect(systemYaml).toContain('i2c_address: 0x76');
    expect(systemYaml).toContain('kind: "i2c_device"');
    expect(systemYaml).toContain('device_type: "bme280"');
  });

  it('maps wired SPI displays (ILI9341) into spi external_devices + board_io', () => {
    const diagram: Diagram = {
      version: 1,
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'nucleo-l476rg', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'tft', type: 'ili9341', x: 200, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PB3' }, to: { part: 'tft', pin: 'SCK' }, color: '#5BD8FF' },
        { from: { part: 'mcu', pin: 'PB5' }, to: { part: 'tft', pin: 'MOSI' }, color: '#B07BFF' },
        { from: { part: 'mcu', pin: 'PA4' }, to: { part: 'tft', pin: 'CS' }, color: '#FFD166' },
      ],
    };

    const { systemYaml } = diagramToConfig(diagram);

    expect(systemYaml).toContain('id: "tft"');
    expect(systemYaml).toContain('type: "ili9341"');
    expect(systemYaml).toContain('connection: "spi1"');
    expect(systemYaml).toContain('cs_pin: "PA4"');
    expect(systemYaml).toMatch(/kind: "spi_device"[\s\S]*device_type: "ili9341"/);
  });

  it('maps wired UART devices (NEO-6M GPS) into uart external_devices + board_io', () => {
    const diagram: Diagram = {
      version: 1,
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'nucleo-l476rg', x: 0, y: 0, rotate: 0, attrs: {} },
        { id: 'gps', type: 'neo6m-gps', x: 200, y: 100, rotate: 0, attrs: {} },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PA9' }, to: { part: 'gps', pin: 'RX' }, color: '#06D6A0' },
        { from: { part: 'mcu', pin: 'PA10' }, to: { part: 'gps', pin: 'TX' }, color: '#118AB2' },
      ],
    };

    const { systemYaml } = diagramToConfig(diagram);

    expect(systemYaml).toContain('id: "gps"');
    expect(systemYaml).toContain('type: "neo6m-gps"');
    expect(systemYaml).toContain('connection: "uart1"');
    expect(systemYaml).toContain('kind: "uart_device"');
    expect(systemYaml).toContain('device_type: "neo6m-gps"');
  });
});
