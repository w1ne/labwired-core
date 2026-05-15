import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const tokens = readFileSync(resolve(__dirname, './tokens.css'), 'utf-8');

const REQUIRED_TOKENS = [
  '--lw-bg-base', '--lw-bg-surface', '--lw-bg-elevated', '--lw-bg-canvas',
  '--lw-fg-primary', '--lw-fg-secondary', '--lw-fg-tertiary',
  '--lw-border', '--lw-border-strong', '--lw-highlight',
  '--lw-accent', '--lw-accent-hover', '--lw-accent-soft',
  '--lw-magenta', '--lw-magenta-soft',
  '--lw-success', '--lw-warning', '--lw-danger',
  '--lw-pin-power', '--lw-pin-gnd',
  '--lw-pin-i2c-sda', '--lw-pin-i2c-scl',
  '--lw-pin-spi-mosi', '--lw-pin-spi-miso', '--lw-pin-spi-sck',
  '--lw-pin-data',
];

describe('design tokens', () => {
  it('exports every token named in the design spec', () => {
    for (const token of REQUIRED_TOKENS) {
      expect(tokens).toContain(`${token}:`);
    }
  });

  it('defines tokens under :root', () => {
    expect(tokens).toMatch(/:root\s*{/);
  });
});
