/**
 * Kernel totality tests (Fix 2).
 *
 * composeDiagnostics(), erc(), and compile() must never throw on malformed or
 * partial input — they return structured error results instead.
 */

import { describe, expect, it } from 'vitest';
import { composeDiagnostics, erc, compile } from '../src';
import type { DiagramV2 } from '../src/schema';

describe('composeDiagnostics() totality', () => {
  it('null input returns SCHEMA_MALFORMED, does not throw', () => {
    expect(() => composeDiagnostics(null as never)).not.toThrow();
    const result = composeDiagnostics(null as never);
    expect(result.ok).toBe(false);
    expect(result.error_count).toBeGreaterThan(0);
    const codes = result.diagnostics.map((d) => d.code);
    expect(codes).toContain('SCHEMA_MALFORMED');
  });

  it('non-object input returns SCHEMA_MALFORMED, does not throw', () => {
    expect(() => composeDiagnostics('not an object' as never)).not.toThrow();
    const result = composeDiagnostics('not an object' as never);
    expect(result.ok).toBe(false);
    expect(result.diagnostics.map((d) => d.code)).toContain('SCHEMA_MALFORMED');
  });

  it('v2 diagram WITHOUT wires field does not throw', () => {
    const diagramNoWires = {
      version: 2,
      board: 'esp32-s3-zero',
      parts: [{ id: 'mcu', type: 'esp32-s3-zero' }],
      nets: [],
      connections: [],
      // wires intentionally omitted
    } as unknown as DiagramV2;

    expect(() => composeDiagnostics(diagramNoWires as never)).not.toThrow();
    const result = composeDiagnostics(diagramNoWires as never);
    // Should succeed with no errors (clean diagram, just missing wires field)
    expect(result.error_count).toBe(0);
  });

  it('v1 diagram WITHOUT wires field does not throw', () => {
    const diagramNoWires = {
      board: 'stm32f103',
      parts: [{ id: 'mcu', type: 'stm32-dev' }],
      // wires intentionally omitted
    } as unknown as DiagramV2;

    expect(() => composeDiagnostics(diagramNoWires as never)).not.toThrow();
    // Should process without throwing — outcome depends on content
  });

  it('agent-generated wires with missing pin fields return diagnostics instead of throwing', () => {
    const diagram = {
      board: 'stm32l476',
      parts: [{ id: 'mcu', type: 'mcu' }],
      wires: [
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1' } },
        { from: { part: 'mcu' }, to: { part: 'led1', pin: 'A' } },
      ],
    } as unknown as DiagramV2;

    expect(() => composeDiagnostics(diagram as never)).not.toThrow();
    const result = composeDiagnostics(diagram as never);
    expect(result.ok).toBe(false);
    expect(result.error_count).toBeGreaterThan(0);
  });
});

describe('erc() totality', () => {
  it('null input returns SCHEMA_MALFORMED, does not throw', () => {
    expect(() => erc(null as never)).not.toThrow();
    const diags = erc(null as never);
    expect(diags.length).toBeGreaterThan(0);
    expect(diags.map((d) => d.code)).toContain('SCHEMA_MALFORMED');
  });

  it('undefined input returns SCHEMA_MALFORMED, does not throw', () => {
    expect(() => erc(undefined as never)).not.toThrow();
    const diags = erc(undefined as never);
    expect(diags.map((d) => d.code)).toContain('SCHEMA_MALFORMED');
  });

  it('SCHEMA_MALFORMED from erc() has severity error', () => {
    const diags = erc(null as never);
    const malformed = diags.find((d) => d.code === 'SCHEMA_MALFORMED');
    expect(malformed?.severity).toBe('error');
  });
});

describe('compile() totality via ERC gate', () => {
  it('null input returns ok:false with SCHEMA_MALFORMED, does not throw', () => {
    expect(() => compile(null as never)).not.toThrow();
    const result = compile(null as never);
    expect(result.ok).toBe(false);
    expect(result.diagnostics.map((d) => d.code)).toContain('SCHEMA_MALFORMED');
  });
});
