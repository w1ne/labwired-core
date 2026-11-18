#!/usr/bin/env node
// SPDX-License-Identifier: MIT
// Run portable firmware.wasm against the compiled LabWired Wasm runtime.
// No CPU, target ELF, or MMIO address space participates.
//
// Browser mode (required dependency is explicit; never silently skip):
//   wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web
//   LABWIRED_PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs \
//     node scripts/test_browser_flex.mjs --browser
// Optional Node mode uses crates/wasm/pkg from wasm-pack --target nodejs.
// Firmware: crates/wasm/tests/fixtures/flex-mixed-signal/build.sh
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { createServer } from 'node:http';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, resolve, extname, sep, relative } from 'node:path';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const fixturesRoot = resolve(root, 'crates/wasm/tests/fixtures');
const argValue = (flag, fallback) => {
  const index = process.argv.indexOf(flag);
  return index < 0 ? fallback : process.argv[index + 1];
};
const fixture = resolve(root, argValue('--fixture', 'crates/wasm/tests/fixtures/flex-mixed-signal'));
assert(fixture.startsWith(fixturesRoot + sep), 'The fixture must stay under crates/wasm/tests/fixtures.');
const checkName = argValue('--check', 'runPortableChecks');
const checkModulePath = resolve(fixture, 'checks.mjs');
assert(existsSync(checkModulePath), `Missing ${checkModulePath}.`);
const checkModuleUrl = pathToFileURL(checkModulePath).href;
const fixtureUrl = `/${relative(root, fixture).split(sep).join('/')}/`;
const nodeChecks = await import(checkModuleUrl);
assert(typeof nodeChecks[checkName] === 'function', `Missing ${checkName} in ${checkModulePath}.`);
let result;
if (process.argv.includes('--browser')) {
  const playwrightPath = process.env.LABWIRED_PLAYWRIGHT_MODULE;
  assert(playwrightPath && existsSync(playwrightPath), 'Set LABWIRED_PLAYWRIGHT_MODULE to an installed Playwright module; browser validation cannot be skipped.');
  const pkg = resolve(root, 'crates/wasm/pkg-flex-web');
  assert(existsSync(resolve(pkg, 'labwired_wasm.js')), 'Build crates/wasm with --target web --out-dir pkg-flex-web first.');
  const { chromium } = await import(pathToFileURL(resolve(playwrightPath)).href);
  const launchOptions = { headless: true };
  if (process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH) {
    launchOptions.executablePath = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH;
  }
  const server = createServer((req, res) => {
    const pathname = new URL(req.url, 'http://127.0.0.1').pathname;
    if (pathname === '/') { res.setHeader('Content-Type', 'text/html'); res.end('<!doctype html><title>LabWired Flex browser test</title>'); return; }
    const path = resolve(root, `.${pathname}`);
    if (![pkg, fixture].some((allowed) => path.startsWith(allowed + sep))) { res.writeHead(404); res.end(); return; }
    try {
      const contents = readFileSync(path);
      res.setHeader('Content-Type', ({ '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json' })[extname(path)] ?? 'text/plain');
      res.end(contents);
    } catch { res.writeHead(404); res.end(); }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  let browser;
  try {
    browser = await chromium.launch(launchOptions);
    const page = await browser.newPage();
    const url = `http://127.0.0.1:${server.address().port}`;
    await page.goto(url);
    result = await page.evaluate(async ({ base, checkName }) => {
      const [{ default: init, WasmFlexRuntime }, checks, yaml, firmware, expected] = await Promise.all([
        import('/crates/wasm/pkg-flex-web/labwired_wasm.js'),
        import(base + 'checks.mjs'),
        fetch(base + 'system.yaml').then((r) => r.text()),
        fetch(base + 'firmware.wasm').then((r) => r.arrayBuffer()),
        fetch(base + 'native.json').then((r) => r.json()),
      ]);
      await init();
      return checks[checkName](WasmFlexRuntime, yaml, new Uint8Array(firmware), expected);
    }, { base: fixtureUrl, checkName });
    result.browser = await browser.version();
  } finally {
    if (browser) await browser.close();
    await new Promise((resolve) => server.close(resolve));
  }
} else {
  const pkg = resolve(root, 'crates/wasm/pkg/labwired_wasm.js');
  assert(existsSync(pkg), `Missing ${pkg}; build the nodejs Wasm package first, or use --browser.`);
  const { WasmFlexRuntime } = await import(pkg);
  const read = (name) => readFileSync(resolve(fixture, name));
  result = nodeChecks[checkName](WasmFlexRuntime, read('system.yaml').toString(), read('firmware.wasm'), JSON.parse(read('native.json').toString()));
  result.host = 'Node';
}
console.log(JSON.stringify(result, null, 2));
