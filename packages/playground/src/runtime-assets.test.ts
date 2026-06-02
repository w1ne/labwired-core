import { describe, expect, it } from 'vitest';
import { versionRuntimeAssetUrl } from './runtime-assets';

describe('versionRuntimeAssetUrl', () => {
  it('adds a build version query to runtime firmware assets', () => {
    expect(versionRuntimeAssetUrl('/wasm/demo-nokia5110-invaders-lab.elf', 1234)).toBe(
      '/wasm/demo-nokia5110-invaders-lab.elf?v=1234',
    );
  });

  it('preserves existing query parameters', () => {
    expect(versionRuntimeAssetUrl('/wasm/demo.elf?kind=demo', 1234)).toBe(
      '/wasm/demo.elf?kind=demo&v=1234',
    );
  });
});
