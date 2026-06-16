import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

const script = path.join(process.cwd(), 'scripts/verify-production-env.mjs');

describe('production env guard', () => {
  it('rejects production builds without Clerk publishable key unless auth is explicitly disabled', () => {
    expect(() => execFileSync(process.execPath, [script], {
      env: {
        PATH: process.env.PATH,
        NODE_ENV: 'production',
      },
      stdio: 'pipe',
    })).toThrow(/VITE_CLERK_PUBLISHABLE_KEY/);
  });

  it('accepts production builds with Clerk publishable key', () => {
    expect(() => execFileSync(process.execPath, [script], {
      env: {
        PATH: process.env.PATH,
        NODE_ENV: 'production',
        VITE_CLERK_PUBLISHABLE_KEY: 'pk_live_test',
      },
      stdio: 'pipe',
    })).not.toThrow();
  });
});
