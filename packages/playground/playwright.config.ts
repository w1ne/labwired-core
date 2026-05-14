import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: false,
  timeout: 30_000,
  use: {
    baseURL: 'http://localhost:5173',
    headless: true,
  },
  webServer: {
    command: 'npm run dev -- --port 5173 --host 127.0.0.1',
    url: 'http://localhost:5173',
    timeout: 60_000,
    reuseExistingServer: !process.env.CI,
  },
});
