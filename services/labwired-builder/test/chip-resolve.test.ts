import { describe, it, expect } from 'vitest';
import { resolveChipInManifest } from '../src/run';
import { CHIP_YAMLS } from '../../../packages/board-config/src/chip-yamls';

const knownId = Object.keys(CHIP_YAMLS)[0]; // a guaranteed-valid id

describe('resolveChipInManifest', () => {
  it('resolves a bare chip id to its bundled YAML and rewrites the field', () => {
    const sys = `name: "t"\nchip: "${knownId}"\nboard_io: []\n`;
    const out = resolveChipInManifest(sys);
    expect(out.chipYaml).toBe(CHIP_YAMLS[knownId]);
    expect(out.systemYaml).toMatch(/^chip:\s*"chip\.yaml"/m);
  });

  it('throws a listing error on an unknown bare id', () => {
    const sys = `name: "t"\nchip: "totally-not-a-chip"\nboard_io: []\n`;
    expect(() => resolveChipInManifest(sys)).toThrow(/unknown chip id .*Known chip ids:/s);
  });

  it('leaves a path-style chip untouched', () => {
    const sys = `name: "t"\nchip: "../../configs/chips/stm32f103.yaml"\nboard_io: []\n`;
    const out = resolveChipInManifest(sys);
    expect(out.chipYaml).toBeUndefined();
    expect(out.systemYaml).toBe(sys);
  });

  it('honors an explicit chipYaml override and rewrites the inline placeholder', () => {
    const sys = `name: "t"\nchip: "inline"\nboard_io: []\n`;
    const out = resolveChipInManifest(sys, 'name: custom\n');
    expect(out.chipYaml).toBe('name: custom\n');
    expect(out.systemYaml).toMatch(/^chip:\s*"chip\.yaml"/m);
  });
});
