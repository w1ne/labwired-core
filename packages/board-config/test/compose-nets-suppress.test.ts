/**
 * Fix 3: suppress wire-heuristic false warnings on nets-canonical diagrams.
 *
 * COMPONENT_DANGLING is a wire-count heuristic: it fires when a board_io part
 * has zero MCU wire connections.  On a nets-canonical v2 diagram that part IS
 * connected via connections[] — the legacy check just doesn't see it.
 *
 * Contract:
 *   - A part connected ONLY via connections[] must NOT produce COMPONENT_DANGLING.
 *   - A genuinely dangling part (zero wires AND zero connections entries) MUST
 *     still produce COMPONENT_DANGLING.
 */

import { describe, expect, it } from 'vitest';
import { composeDiagnostics } from '../src';
import type { DiagramV2 } from '../src/schema';

// BME280 wired only via v2 connections (nets-canonical diagram).
// This is the style the SpiceDispenser uses; no wires at all.
const bme280NetsDiagram: DiagramV2 = {
  version: 2,
  board: 'esp32-s3-zero',
  parts: [
    { id: 'mcu', type: 'esp32-s3-zero' },
    { id: 'b1', type: 'bme280', attrs: {} },
    { id: 'r1', type: 'resistor' },  // pull-up for SDA
    { id: 'r2', type: 'resistor' },  // pull-up for SCL
  ],
  nets: [
    { name: 'V3',  kind: 'power',  voltage: 3.3 },
    { name: 'GND', kind: 'power',  voltage: 0 },
    { name: 'SDA', kind: 'signal', protocol: 'i2c_sda' },
    { name: 'SCL', kind: 'signal', protocol: 'i2c_scl' },
  ],
  connections: [
    ['mcu:3V3',   'V3'],
    ['mcu:GND',   'GND'],
    ['mcu:GPIO8', 'SDA'],
    ['mcu:GPIO9', 'SCL'],
    ['b1:VCC',    'V3'],
    ['b1:GND',    'GND'],
    ['b1:SDA',    'SDA'],
    ['b1:SCL',    'SCL'],
    // Pull-up resistors satisfy I2C_NO_PULLUP
    ['r1:1', 'SDA'], ['r1:2', 'V3'],
    ['r2:1', 'SCL'], ['r2:2', 'V3'],
  ],
  wires: [],
};

// Genuinely dangling part: in parts[] but zero connections AND zero wires.
const withDanglingDiagram: DiagramV2 = {
  ...bme280NetsDiagram,
  parts: [
    ...bme280NetsDiagram.parts,
    { id: 'orphan_led', type: 'led' },
  ],
  // No connections or wires for orphan_led
};

describe('composeDiagnostics — Fix 3: suppress COMPONENT_DANGLING for nets-connected parts', () => {
  it('bme280 connected via connections[] only → no COMPONENT_DANGLING', () => {
    const result = composeDiagnostics(bme280NetsDiagram as never);
    const codes = result.diagnostics.map((d) => d.code);
    expect(codes, 'bme280 is in connections[], should not be flagged as dangling')
      .not.toContain('COMPONENT_DANGLING');
  });

  it('bme280 nets-canonical diagram → ok:true (no errors)', () => {
    const result = composeDiagnostics(bme280NetsDiagram as never);
    expect(result.ok, `Expected ok:true but got errors: ${JSON.stringify(result.diagnostics.filter((d) => d.severity === 'error'))}`).toBe(true);
  });

  it('genuinely dangling part (no wires, no connections) → COMPONENT_DANGLING still fires', () => {
    const result = composeDiagnostics(withDanglingDiagram as never);
    const danglingDiags = result.diagnostics.filter(
      (d) => d.code === 'COMPONENT_DANGLING',
    );
    // orphan_led has no wires and no connections entry → should be flagged
    const orphanFlagged = danglingDiags.some(
      (d) => d.location?.part_id === 'orphan_led',
    );
    expect(orphanFlagged, 'orphan_led has no connections or wires → must still be COMPONENT_DANGLING').toBe(true);
  });

  it('b1 (bme280) is NOT flagged as dangling even though orphan_led is', () => {
    const result = composeDiagnostics(withDanglingDiagram as never);
    const bme280Dangling = result.diagnostics.some(
      (d) => d.code === 'COMPONENT_DANGLING' && d.location?.part_id === 'b1',
    );
    expect(bme280Dangling, 'b1 is in connections[], must not be dangling').toBe(false);
  });
});
