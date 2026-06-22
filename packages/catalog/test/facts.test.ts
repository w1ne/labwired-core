import { describe, it, expect } from 'vitest';
import { execFileSync } from 'node:child_process';

describe('catalog-facts generation', () => {
  it('src/catalog-facts.json is up to date with the generator', () => {
    expect(() =>
      execFileSync('npm', ['run', 'check:facts'], {
        cwd: new URL('..', import.meta.url),
        stdio: 'pipe',
      }),
    ).not.toThrow();
  });
});
