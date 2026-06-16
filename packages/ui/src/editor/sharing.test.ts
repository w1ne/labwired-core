import { describe, expect, it } from 'vitest';
import { decodeProject, normalizeSharedDiagram } from './sharing';

const failingAgentHash =
  'reyJkIjp7ImJvYXJkIjoic3RtMzJsNDc2IiwicGFydHMiOlt7ImlkIjoibWN1IiwidHlwZSI6Im1jdSIsImxhYmVsIjoiU1RNMzJMNDc2In0seyJpZCI6ImxlZDEiLCJ0eXBlIjoibGVkIiwibGFiZWwiOiJMRUQiLCJjb2xvciI6ImdyZWVuIn1dLCJ3aXJlcyI6W3siZnJvbSI6eyJwYXJ0IjoibWN1IiwicGluIjoiUEE1In0sInRvIjp7InBhcnQiOiJsZWQxIiwicGluIjoiQSJ9fV19LCJzIjoiIn0';

describe('shared project decoding', () => {
  it('normalizes compact agent diagrams into editor-safe diagrams', async () => {
    const project = await decodeProject(failingAgentHash);

    expect(project).not.toBeNull();
    expect(project!.diagram).toMatchObject({
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

  it('preserves explicit editor fields while filling missing defaults', () => {
    const diagram = normalizeSharedDiagram({
      version: 1,
      board: 'stm32l476',
      parts: [
        {
          id: 'led1',
          type: 'led',
          x: 12,
          y: 34,
          rotate: 90,
          scale: 1.5,
          attrs: { color: 'blue' },
        },
      ],
      wires: [],
    });

    expect(diagram).toMatchObject({
      version: 1,
      parts: [
        { id: 'led1', type: 'led', x: 12, y: 34, rotate: 90, scale: 1.5, attrs: { color: 'blue' } },
      ],
    });
  });
});
