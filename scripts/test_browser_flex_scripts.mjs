#!/usr/bin/env node
// SPDX-License-Identifier: MIT
// Run every Flex `test-*.yaml` script in a real browser and in `labwired test`,
// and require the same result and the same instrument capture from both.
// No CPU, target ELF, or MMIO address space participates.
//
//   wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web
//   LABWIRED_PLAYWRIGHT_MODULE=/absolute/path/to/playwright/index.mjs \
//     node scripts/test_browser_flex_scripts.mjs --cli /path/to/labwired \
//     > crates/wasm/tests/fixtures/browser-flex-scripts.json
//
// `--fixture <dir>` limits the run to one fixture directory. The receipt test
// in crates/wasm/src/flex_tests.rs needs the run of all fixtures.
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { basename, dirname, extname, join, relative, resolve, sep } from 'node:path';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const fixturesRoot = resolve(root, 'crates/wasm/tests/fixtures');
const argValue = (flag) => {
  const index = process.argv.indexOf(flag);
  return index < 0 ? undefined : process.argv[index + 1];
};
const cli = argValue('--cli');
assert(cli && existsSync(cli), 'Set --cli to a built labwired binary; the native reference cannot be skipped.');
const playwrightPath = process.env.LABWIRED_PLAYWRIGHT_MODULE;
assert(playwrightPath && existsSync(playwrightPath), 'Set LABWIRED_PLAYWRIGHT_MODULE to an installed Playwright module; browser validation cannot be skipped.');
const pkg = resolve(root, 'crates/wasm/pkg-flex-web');
assert(existsSync(resolve(pkg, 'labwired_wasm.js')), 'Build crates/wasm with --target web --out-dir pkg-flex-web first.');

const onlyFixture = argValue('--fixture');
const fixtures = (onlyFixture ? [resolve(root, onlyFixture)] : readdirSync(fixturesRoot)
  .filter((name) => name.startsWith('flex-'))
  .map((name) => join(fixturesRoot, name)))
  .filter((dir) => dir.startsWith(fixturesRoot + sep));
const scripts = fixtures.flatMap((dir) => readdirSync(dir)
  .filter((name) => /^test-.*\.yaml$/.test(name))
  .sort()
  .map((name) => ({ dir, name })))
  .sort((a, b) => a.dir.localeCompare(b.dir) || a.name.localeCompare(b.name));
assert(scripts.length > 0, 'No test-*.yaml script found.');

const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
// The inputs a script names, as written. The Flex script schema keeps both on
// one line each under `inputs:`.
const scriptInput = (text, key) => {
  const match = text.match(new RegExp(`^\\s+${key}:\\s*(\\S+)\\s*$`, 'm'));
  assert(match, `script has no inputs.${key}`);
  return match[1].replace(/^["']|["']$/g, '');
};
// The CLI result names its inputs by resolved path, the browser by the names
// in the script. Every other line must be the same text.
const withLabels = (resultText, labels) => resultText
  .replace(/^ {4}"script": .*,$/m, `    "script": ${JSON.stringify(labels.script)},`)
  .replace(/^ {4}"flex": .*,$/m, `    "flex": ${JSON.stringify(labels.flex)},`)
  .replace(/^ {4}"firmware": .*,$/m, `    "firmware": ${JSON.stringify(labels.firmware)},`);
// Trace values: the browser gives 64-bit integers as BigInt or Number and f32
// samples widened to f64; the CLI file prints the short f32 form.
const canonical = (v) => typeof v === 'bigint' ? Number(v)
  : typeof v === 'number' ? (Number.isInteger(v) ? v : Math.fround(v))
    : Array.isArray(v) ? v.map(canonical)
      : v !== null && typeof v === 'object'
        ? Object.fromEntries(Object.keys(v).sort().map((key) => [key, canonical(v[key])])) : v;

const server = createServer((req, res) => {
  const pathname = new URL(req.url, 'http://127.0.0.1').pathname;
  if (pathname === '/') { res.setHeader('Content-Type', 'text/html'); res.end('<!doctype html><title>LabWired Flex script test</title>'); return; }
  const path = resolve(root, `.${pathname}`);
  if (![pkg, fixturesRoot].some((allowed) => path.startsWith(allowed + sep))) { res.writeHead(404); res.end(); return; }
  try {
    const contents = readFileSync(path);
    res.setHeader('Content-Type', ({ '.js': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json' })[extname(path)] ?? 'text/plain');
    res.end(contents);
  } catch { res.writeHead(404); res.end(); }
});
await new Promise((done) => server.listen(0, '127.0.0.1', done));

const { chromium } = await import(pathToFileURL(resolve(playwrightPath)).href);
const launchOptions = { headless: true };
if (process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH) {
  launchOptions.executablePath = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH;
}
const out = mkdtempSync(join(tmpdir(), 'labwired-flex-scripts-'));
const rows = [];
let browser;
let browserVersion;
try {
  browser = await chromium.launch(launchOptions);
  browserVersion = await browser.version();
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  await page.evaluate(async () => {
    const module = await import('/crates/wasm/pkg-flex-web/labwired_wasm.js');
    await module.default();
    globalThis.labwired = module;
  });
  for (const { dir, name } of scripts) {
    const fixture = basename(dir);
    const scriptBytes = readFileSync(join(dir, name));
    const scriptText = scriptBytes.toString();
    const labels = {
      script: name,
      flex: scriptInput(scriptText, 'flex'),
      firmware: scriptInput(scriptText, 'firmware'),
    };
    const nativeDir = join(out, fixture, name);
    const nativeStarted = performance.now();
    const native = spawnSync(cli, ['test', '--script', join(dir, name), '--output-dir', nativeDir], { encoding: 'utf8' });
    const nativeMs = performance.now() - nativeStarted;
    assert(existsSync(join(nativeDir, 'result.json')), `${fixture}/${name}: labwired test wrote no result.json (exit ${native.status}): ${native.stderr}`);
    const nativeResult = withLabels(readFileSync(join(nativeDir, 'result.json'), 'utf8'), labels);
    const nativeTrace = JSON.parse(readFileSync(join(nativeDir, 'flex-trace.json'), 'utf8'));

    const base = `/${relative(root, dir).split(sep).join('/')}/`;
    const ran = await page.evaluate(async ({ base, name, labels }) => {
      const { WasmFlexTestRun } = globalThis.labwired;
      const [script, manifest, firmware] = await Promise.all([
        fetch(base + name).then((r) => r.text()),
        fetch(base + labels.flex).then((r) => r.text()),
        fetch(base + labels.firmware).then((r) => r.arrayBuffer()),
      ]);
      const startedAt = performance.now();
      const run = WasmFlexTestRun.run(name, script, manifest, new Uint8Array(firmware));
      const ms = performance.now() - startedAt;
      try {
        const result = run.result_json();
        // The existing instrument methods read the capture of the script run.
        const runtime = run.take_runtime();
        try {
          const plain = (v) => JSON.parse(JSON.stringify(v, (_, x) => typeof x === 'bigint' ? Number(x) : x));
          return {
            result,
            ms,
            digital: plain(runtime.digital_trace_snapshot(0)),
            analog: plain(runtime.analog_trace_snapshot(0)),
            analogSeries: plain(runtime.analog_series()),
            i2c: plain(runtime.i2c_trace_snapshot(0)),
            spi: plain(runtime.spi_trace_snapshot(0)),
            // `null` or `undefined` for a manifest without a console.
            console: runtime.console_capture() == null ? undefined : plain(runtime.console_capture()),
          };
        } finally {
          runtime.free();
        }
      } finally {
        run.free();
      }
    }, { base, name, labels });

    assert.equal(ran.result, nativeResult, `${fixture}/${name}: the browser result differs from labwired test`);
    assert.deepEqual(canonical(ran.digital), canonical(nativeTrace.digital), `${fixture}/${name}: the browser logic-analyzer capture differs from labwired test`);
    assert.deepEqual(canonical(ran.analog), canonical(nativeTrace.analog), `${fixture}/${name}: the browser oscilloscope capture differs from labwired test`);
    assert.deepEqual(canonical(ran.analogSeries), canonical(nativeTrace.analog_series), `${fixture}/${name}: the browser run-long analog series differs from labwired test`);
    // The CLI writes the I2C record only for a manifest with a bus.
    if (ran.i2c.buses.length > 0 || nativeTrace.i2c !== undefined) {
      assert.deepEqual(canonical(ran.i2c), canonical(nativeTrace.i2c), `${fixture}/${name}: the browser I2C transfer record differs from labwired test`);
    }
    // The CLI writes the SPI record only for a manifest with a bus.
    if (ran.spi.buses.length > 0 || nativeTrace.spi !== undefined) {
      assert.deepEqual(canonical(ran.spi), canonical(nativeTrace.spi), `${fixture}/${name}: the browser SPI transfer record differs from labwired test`);
    }
    // The CLI writes the console record only for a manifest with a console.
    assert.deepEqual(canonical(ran.console), canonical(nativeTrace.console), `${fixture}/${name}: the browser console record differs from labwired test`);
    const result = JSON.parse(ran.result);
    assert.equal(result.status, 'pass', `${fixture}/${name}: status ${result.status} (${result.stop_reason})`);
    assert.equal(native.status, 0, `${fixture}/${name}: labwired test exit ${native.status}`);
    rows.push({
      fixture,
      script: name,
      script_sha256: sha256(scriptBytes),
      manifest_sha256: sha256(readFileSync(join(dir, labels.flex))),
      firmware_sha256: result.config.firmware_sha256,
      status: result.status,
      stop_reason: result.stop_reason,
      time_ns: result.time_ns,
      steps_executed: result.steps_executed,
      suspended_steps: result.steps.filter((step) => step.suspended).length,
      assertions: result.assertions.length,
      digital_edges: ran.digital.edges.length,
      analog_samples: ran.analog.samples.length,
      i2c_transfers: ran.i2c.transfers.length,
      spi_transfers: ran.spi.transfers.length,
      console_bytes: ran.console?.bytes ?? 0,
      // SHA-256 of the result text both hosts produced.
      result_sha256: sha256(ran.result),
      measured_native_ms: Math.round(nativeMs),
      measured_browser_ms: Math.round(ran.ms),
    });
  }
} finally {
  if (browser) await browser.close();
  await new Promise((done) => server.close(done));
  rmSync(out, { recursive: true, force: true });
}

console.log(JSON.stringify({
  status: 'passed',
  checks: ['result-text-equality', 'logic-analyzer-capture-equality', 'oscilloscope-capture-equality', 'analog-series-equality', 'i2c-transfer-record-equality', 'spi-transfer-record-equality', 'console-record-equality', 'every-script-passes'],
  execution_path: 'flex_portable_wasm',
  cpu_emulation: false,
  firmware_interpreter: 'wasmi, in labwired-wasm and in the CLI',
  browser: browserVersion,
  browser_driver: 'Playwright',
  core_commit: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
  fixtures: new Set(rows.map((row) => row.fixture)).size,
  scripts: rows,
}, null, 2));
