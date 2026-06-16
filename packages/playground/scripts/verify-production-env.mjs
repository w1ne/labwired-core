import fs from 'node:fs';
import path from 'node:path';

function loadDotEnv(file) {
  if (!fs.existsSync(file)) return {};
  const entries = {};
  for (const rawLine of fs.readFileSync(file, 'utf8').split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith('#')) continue;
    const idx = line.indexOf('=');
    if (idx === -1) continue;
    const key = line.slice(0, idx).trim();
    const value = line.slice(idx + 1).trim().replace(/^['"]|['"]$/g, '');
    entries[key] = value;
  }
  return entries;
}

const envFile = loadDotEnv(path.join(process.cwd(), '.env.local'));
const key = process.env.VITE_CLERK_PUBLISHABLE_KEY ?? envFile.VITE_CLERK_PUBLISHABLE_KEY ?? '';
const authDisabled = (process.env.VITE_DISABLE_AUTH ?? envFile.VITE_DISABLE_AUTH) === 'true';

if (!authDisabled && !key) {
  console.error(
    'VITE_CLERK_PUBLISHABLE_KEY is required for Playground production builds. ' +
      'Set the production Clerk publishable key, or set VITE_DISABLE_AUTH=true only for local/dev builds.',
  );
  process.exit(1);
}
