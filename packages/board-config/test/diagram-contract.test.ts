import { describe, expect, it } from 'vitest';
import { LABWIRED_DIAGRAM_V1_SCHEMA, normalizeLabWiredDiagramV1 } from '../src/index';

describe('LabWired diagram contract', () => {
  it('normalizes compact agent diagrams into the canonical public v1 shape', () => {
    const diagram = normalizeLabWiredDiagramV1({
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'mcu', label: 'STM32L476' },
        { id: 'led1', type: 'led', label: 'LED', color: 'green' },
      ],
      wires: [
        { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' } },
      ],
    });

    expect(diagram).toMatchObject({
      version: 1,
      board: 'stm32l476',
      parts: [
        { id: 'mcu', type: 'mcu', x: 140, y: 140, rotate: 0, attrs: {} },
        { id: 'led1', type: 'led', x: 290, y: 140, rotate: 0, attrs: { color: 'green' } },
      ],
      wires: [
        {
          from: { part: 'mcu', pin: 'PA5' },
          to: { part: 'led1', pin: 'A' },
          color: '#e83e8c',
        },
      ],
    });
  });

  it('exposes a versioned JSON schema requiring editor-safe fields', () => {
    expect(LABWIRED_DIAGRAM_V1_SCHEMA).toMatchObject({
      $id: 'https://labwired.com/schemas/diagram-v1.json',
      type: 'object',
      required: ['version', 'board', 'parts', 'wires'],
      properties: {
        version: { const: 1 },
        parts: {
          items: {
            required: ['id', 'type', 'x', 'y', 'rotate', 'attrs'],
          },
        },
        wires: {
          items: {
            required: ['from', 'to', 'color'],
          },
        },
      },
    });
  });
});
