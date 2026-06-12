/**
 * Regression tests for catalog entries that previously drifted out of sync.
 * Blocking 1: sn74hc165 and iolink-master must be in CATALOG (UNKNOWN_COMPONENT
 *             was the symptom when they were wired in a diagram).
 * Blocking 2: neo6m-gps must have boardIoKind so COMPONENT_DANGLING fires when unwired.
 */
import { describe, it, expect } from 'vitest';
import { diagnoseDiagram } from '../src/legacy-diagnostics';
import { getCatalogPart } from '../src/catalog';

// Minimal diagram helpers
const mcuPart = { id: 'mcu', type: 'esp32-s3-zero' };
const wire = (fromPin: string, toId: string, toPin: string) => ({
  from: { part: 'mcu', pin: fromPin },
  to: { part: toId, pin: toPin },
});

describe('Blocking 1 — sn74hc165 and iolink-master in CATALOG', () => {
  it('sn74hc165 is in the catalog with deviceClass spi_device and boardIoKind spi_device', () => {
    const part = getCatalogPart('sn74hc165');
    expect(part, 'sn74hc165 must be in CATALOG').toBeDefined();
    expect(part!.deviceClass).toBe('spi_device');
    expect(part!.boardIoKind).toBe('spi_device');
  });

  it('iolink-master is in the catalog with deviceClass uart_device and boardIoKind uart_device', () => {
    const part = getCatalogPart('iolink-master');
    expect(part, 'iolink-master must be in CATALOG').toBeDefined();
    expect(part!.deviceClass).toBe('uart_device');
    expect(part!.boardIoKind).toBe('uart_device');
  });

  it('diagnoseDiagram on a wired sn74hc165 does NOT return UNKNOWN_COMPONENT', () => {
    const diagram = {
      board: 'esp32-s3-zero',
      parts: [mcuPart, { id: 'sr1', type: 'sn74hc165' }],
      wires: [wire('GPIO10', 'sr1', 'CLK')],
    };
    const diags = diagnoseDiagram(diagram);
    const codes = diags.map((d) => d.code);
    expect(codes).not.toContain('UNKNOWN_COMPONENT');
  });

  it('diagnoseDiagram on a wired iolink-master does NOT return UNKNOWN_COMPONENT', () => {
    const diagram = {
      board: 'esp32-s3-zero',
      parts: [mcuPart, { id: 'iol1', type: 'iolink-master' }],
      wires: [wire('GPIO4', 'iol1', 'TX')],
    };
    const diags = diagnoseDiagram(diagram);
    const codes = diags.map((d) => d.code);
    expect(codes).not.toContain('UNKNOWN_COMPONENT');
  });
});

describe('Blocking 2 — neo6m-gps boardIoKind restored', () => {
  it('neo6m-gps catalog entry has boardIoKind uart_device', () => {
    const part = getCatalogPart('neo6m-gps');
    expect(part, 'neo6m-gps must be in CATALOG').toBeDefined();
    expect(part!.boardIoKind).toBe('uart_device');
  });

  it('unwired neo6m-gps fires COMPONENT_DANGLING', () => {
    const diagram = {
      board: 'esp32-s3-zero',
      parts: [mcuPart, { id: 'gps1', type: 'neo6m-gps' }],
      wires: [],
    };
    const diags = diagnoseDiagram(diagram);
    const codes = diags.map((d) => d.code);
    expect(codes).toContain('COMPONENT_DANGLING');
  });

  it('wired neo6m-gps does NOT fire COMPONENT_DANGLING', () => {
    const diagram = {
      board: 'esp32-s3-zero',
      parts: [mcuPart, { id: 'gps1', type: 'neo6m-gps' }],
      wires: [wire('GPIO17', 'gps1', 'RX')],
    };
    const diags = diagnoseDiagram(diagram);
    const codes = diags.map((d) => d.code);
    expect(codes).not.toContain('COMPONENT_DANGLING');
  });
});
