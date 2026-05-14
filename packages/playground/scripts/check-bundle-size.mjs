// packages/playground/scripts/check-bundle-size.mjs
import { readdirSync, statSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { gzipSync } from 'node:zlib';
import { readFileSync } from 'node:fs';

const ROOT = resolve(import.meta.dirname, '..', 'dist', 'assets');

try {
  const files = readdirSync(ROOT).filter((f) => f.endsWith('.js') && !f.includes('legacy'));
  console.log('Main bundle JS files:');
  let total = 0;
  for (const file of files) {
    const path = join(ROOT, file);
    const raw = readFileSync(path);
    const gz = gzipSync(raw).length;
    total += gz;
    console.log(`  ${file}: ${(raw.length / 1024).toFixed(1)} KB raw, ${(gz / 1024).toFixed(1)} KB gz`);
  }
  console.log(`Total main JS (gz): ${(total / 1024).toFixed(1)} KB`);
  if (total > 350 * 1024) {
    console.error(`\n⚠️ Main JS bundle exceeds 350 KB gz target. Investigate.`);
    process.exit(0); // do not fail CI, just warn
  } else {
    console.log(`\n✓ Within 350 KB gz target.`);
  }
} catch (err) {
  console.error('Failed to read dist/. Run `npm run build` first.');
  process.exit(1);
}
