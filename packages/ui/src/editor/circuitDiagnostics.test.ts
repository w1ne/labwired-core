import { describe, it, expect } from 'vitest';
import { diagnoseDiagram } from './circuitDiagnostics';
import { validateWireConnection } from './circuitValidation';
import type { Diagram, Wire } from './types';

/** Terse Wire builder — the diagnostics don't read `color`, but the type wants it. */
const w = (fromPart: string, fromPin: string, toPart: string, toPin: string): Wire => ({
  from: { part: fromPart, pin: fromPin },
  to: { part: toPart, pin: toPin },
  color: '#888888',
});


/**
 * The Nokia 5110 Breakout lab powers two peripherals (PCD8544 display + HC-SR04
 * ultrasonic) off the board's single 3V3 rail — mcu.VCC fans out to both, and
 * mcu.GND fans out to both. That is a correct shared power rail, not an
 * overloaded signal pin, so it must not raise a PIN_OVERLOADED error.
 *
 * `board` is the chip id ('stm32l476') — that's what createEmptyDiagram(chipId)
 * stores and what getPinMapping keys on.
 */
const nokia5110Lab: Diagram = {
  version: 1,
  board: 'stm32l476',
  parts: [
    { id: 'mcu', type: 'nucleo-l476rg', x: 0, y: 0, rotate: 0, attrs: {} },
    { id: 'lcd', type: 'pcd8544', x: 500, y: 60, rotate: 0, attrs: {} },
    { id: 'dist', type: 'ultrasonic', x: 500, y: 280, rotate: 0, attrs: {} },
  ],
  wires: [
    w('mcu', 'VCC', 'lcd', 'VCC'),
    w('mcu', 'GND', 'lcd', 'GND'),
    w('mcu', 'PA5', 'lcd', 'CLK'),
    w('mcu', 'PA7', 'lcd', 'DIN'),
    w('mcu', 'PC7', 'lcd', 'DC'),
    w('mcu', 'PB6', 'lcd', 'CE'),
    w('mcu', 'PA9', 'lcd', 'RST'),
    w('mcu', 'VCC', 'dist', 'VCC'),
    w('mcu', 'GND', 'dist', 'GND'),
    w('mcu', 'PA8', 'dist', 'TRIG'),
    w('mcu', 'PB10', 'dist', 'ECHO'),
  ],
};

describe('power-rail fan-out diagnostics (Nokia 5110 lab)', () => {
  it('does not flag a shared VCC/GND rail as PIN_OVERLOADED', () => {
    const overloaded = diagnoseDiagram(nokia5110Lab).filter((d) => d.code === 'PIN_OVERLOADED');
    expect(overloaded).toEqual([]);
  });

  it('still flags a genuine signal-pin double-assignment', () => {
    // PA5 driving the display's clock AND the sensor's trigger IS an overload.
    const bad: Diagram = {
      ...nokia5110Lab,
      wires: [
        ...nokia5110Lab.wires,
        w('mcu', 'PA5', 'dist', 'TRIG'),
      ],
    };
    const overloaded = diagnoseDiagram(bad).filter((d) => d.code === 'PIN_OVERLOADED');
    expect(overloaded.map((d) => d.location?.pin)).toContain('PA5');
  });

  it('does not flag the HC-SR04 (TRIG+ECHO) as BOARDIO_MULTIPLE_WIRES', () => {
    // HC-SR04 is boardIoKind:'button' but exposes two signal pins (TRIG, ECHO),
    // so two MCU connections are correct — not a single-wire overload.
    const multi = diagnoseDiagram(nokia5110Lab).filter((d) => d.code === 'BOARDIO_MULTIPLE_WIRES');
    expect(multi).toEqual([]);
  });

  it('produces no error-severity diagnostics for the shipped lab', () => {
    const errors = diagnoseDiagram(nokia5110Lab).filter((d) => d.severity === 'error');
    expect(errors).toEqual([]);
  });

  it('still flags a true single-signal device wired to two MCU pins', () => {
    // An LED-style single-signal-pin part wired to two GPIOs is a real mistake.
    const withLed: Diagram = {
      ...nokia5110Lab,
      parts: [...nokia5110Lab.parts, { id: 'pir', type: 'pir-sensor', x: 0, y: 0, rotate: 0, attrs: {} }],
      wires: [
        ...nokia5110Lab.wires,
        w('mcu', 'PB0', 'pir', 'OUT'),
        w('mcu', 'PB1', 'pir', 'OUT'),
      ],
    };
    const multi = diagnoseDiagram(withLed).filter((d) => d.code === 'BOARDIO_MULTIPLE_WIRES');
    expect(multi.map((d) => d.location?.part_id)).toContain('pir');
  });
});

/**
 * Interactive (wire-drawing) check. The earlier "component already has an MCU
 * connection" guard fires when the target part is already wired, so to isolate
 * the power-pin exemption we draw onto an as-yet-unwired `dist`: the display is
 * partially wired (VCC + CLK), the sensor has no wires.
 */
const partiallyWired: Diagram = {
  version: 1,
  board: 'stm32l476',
  parts: nokia5110Lab.parts,
  wires: [
    w('mcu', 'VCC', 'lcd', 'VCC'),
    w('mcu', 'PA5', 'lcd', 'CLK'),
  ],
};

describe('power-rail fan-out interactive validation', () => {
  it('allows wiring a second component onto the shared VCC rail', () => {
    const err = validateWireConnection(
      partiallyWired,
      { part: 'mcu', pin: 'VCC' },
      { part: 'dist', pin: 'VCC' },
    );
    expect(err).toBeNull();
  });

  it('still blocks reusing a signal pin already driving another component', () => {
    const err = validateWireConnection(
      partiallyWired,
      { part: 'mcu', pin: 'PA5' },
      { part: 'dist', pin: 'TRIG' },
    );
    expect(err).toMatch(/already assigned/i);
  });
});

describe('logic analyzer probe wiring', () => {
  const iolinkWithAnalyzer: Diagram = {
    version: 1,
    board: 'stm32l476',
    parts: [
      { id: 'mcu', type: 'nucleo-l476rg', x: 0, y: 0, rotate: 0, attrs: {} },
      { id: 'iolink_master', type: 'iolink-master', x: 520, y: 300, rotate: 0, attrs: {} },
      { id: 'analyzer', type: 'logic-analyzer', x: 360, y: 300, rotate: 0, attrs: { decoder: 'auto' } },
    ],
    wires: [
      w('mcu', 'PA2', 'iolink_master', 'RX'),
    ],
  };

  it('allows a probe channel to tap an already-wired IO-Link endpoint', () => {
    const err = validateWireConnection(
      iolinkWithAnalyzer,
      { part: 'analyzer', pin: 'CH0' },
      { part: 'iolink_master', pin: 'RX' },
    );
    expect(err).toBeNull();
  });

  it('keeps analyzer probe wires out of board-IO structural diagnostics', () => {
    const diagram: Diagram = {
      ...iolinkWithAnalyzer,
      wires: [
        ...iolinkWithAnalyzer.wires,
        w('analyzer', 'CH0', 'iolink_master', 'RX'),
      ],
    };
    const errors = diagnoseDiagram(diagram).filter((d) => d.severity === 'error');
    expect(errors).toEqual([]);
  });
});
