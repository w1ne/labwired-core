#!/usr/bin/env node
// SPDX-License-Identifier: MIT
// Run `adapter: ngspice` in a real browser: the web build of labwired-wasm,
// a real ngspice WASM build and the host's JavaScript exchange callback. Both
// browser constructors are exercised with the committed LDR example, and the
// browser `labwired test` runner with portable mixed-signal firmware and the
// source-pinned HeyPCB LDR project:
//   machine:    WasmSimulator.new_from_config_with_ngspice_callback (F401 blinky)
//   flex:       WasmFlexRuntime.new_from_yaml_with_ngspice_callback
//   flex test:  WasmFlexTestRun.run_with_ngspice_callback
//               (crates/wasm/tests/fixtures/flex-mixed-signal, system-ngspice.yaml)
// Every dependency is explicit; a missing one fails, it is never skipped.
//
//   wasm-pack build crates/wasm --target web --dev --out-dir pkg-flex-web
//   LABWIRED_PLAYWRIGHT_MODULE=/abs/playwright/index.mjs \
//   LABWIRED_NGSPICE_HOST_MODULE=/abs/ngspice-runtime.mjs \
//   LABWIRED_NGSPICE_WASM_DIR=/abs/dir/with/ngspice.mjs+ngspice.wasm \
//     node scripts/test_browser_ngspice.mjs
//
// The host module must export `createBrowserNgspiceExchange(models, factory)`,
// `prepareBrowserFlexNgspiceModels(flexYaml, netlists)` and
// `loadNgspiceFactory(glueUrl, wasmUrl)` (the LabWired app's
// packages/board-config/src/ngspice-runtime.ts, bundled as an ES module).
// Exit status is non-zero when any case fails.
import { readFileSync, existsSync } from 'node:fs';
import { createServer } from 'node:http';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, resolve, extname, sep } from 'node:path';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const need = (name) => {
  const value = process.env[name];
  if (!value || !existsSync(value)) {
    console.error(`${name} must name an existing path; browser ngspice validation cannot be skipped.`);
    process.exit(2);
  }
  return resolve(value);
};
const playwright = need('LABWIRED_PLAYWRIGHT_MODULE');
const hostModule = need('LABWIRED_NGSPICE_HOST_MODULE');
const wasmDir = need('LABWIRED_NGSPICE_WASM_DIR');
const pkg = resolve(root, 'crates/wasm/pkg-flex-web');
if (!existsSync(resolve(pkg, 'labwired_wasm.js'))) {
  console.error('Build crates/wasm with --target web --out-dir pkg-flex-web first.');
  process.exit(2);
}
const routes = {
  '/pkg/': pkg,
  '/ngspice/': wasmDir,
  '/example/': resolve(root, 'examples/cosim-ngspice-ldr'),
  '/mixed/': resolve(root, 'crates/wasm/tests/fixtures/flex-mixed-signal'),
  '/heypcb-ldr/': resolve(root, 'crates/wasm/tests/fixtures/flex-heypcb-ldr-network'),
};
const files = {
  '/host.mjs': hostModule,
  '/chip.yaml': resolve(root, 'configs/chips/stm32f401.yaml'),
  '/firmware.elf': resolve(root, 'tests/fixtures/stm32f401-blinky.elf'),
};
const types = { '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm' };
const server = createServer((req, res) => {
  const pathname = new URL(req.url, 'http://127.0.0.1').pathname;
  if (pathname === '/') { res.setHeader('Content-Type', 'text/html'); res.end('<!doctype html><title>ngspice</title>'); return; }
  let path = files[pathname];
  for (const [prefix, dir] of Object.entries(routes)) {
    if (!path && pathname.startsWith(prefix)) {
      const candidate = resolve(dir, `.${pathname.slice(prefix.length - 1)}`);
      if (candidate.startsWith(dir + sep)) path = candidate;
    }
  }
  try {
    const contents = readFileSync(path);
    res.setHeader('Content-Type', types[extname(path)] ?? 'text/plain');
    res.end(contents);
  } catch { res.writeHead(404); res.end(); }
});
await new Promise((done) => server.listen(0, '127.0.0.1', done));

const { chromium } = await import(pathToFileURL(playwright).href);
const launch = { headless: true };
if (process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH) launch.executablePath = process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH;
let browser;
let report;
try {
  browser = await chromium.launch(launch);
  const page = await browser.newPage();
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  report = await page.evaluate(runCases);
  report.browser = await browser.version();
} finally {
  if (browser) await browser.close();
  await new Promise((done) => server.close(done));
}
console.log(JSON.stringify(report, null, 2));
process.exit(report.cases.every((c) => c.pass) ? 0 : 1);

// Runs inside the page. Expected values come from the circuit equations in
// examples/cosim-ngspice-ldr/ldr.cir, as in the native core tests.
async function runCases() {
  const [wasm, host, systemYaml, flexYaml, netlist, chipYaml, firmware] = await Promise.all([
    import('/pkg/labwired_wasm.js'),
    import('/host.mjs'),
    fetch('/example/system.yaml').then((r) => r.text()),
    fetch('/example/flex-ldr.yaml').then((r) => r.text()),
    fetch('/example/ldr.cir').then((r) => r.text()),
    fetch('/chip.yaml').then((r) => r.text()),
    fetch('/firmware.elf').then((r) => r.arrayBuffer()),
  ]);
  await wasm.default();
  const factory = await host.loadNgspiceFactory('/ngspice/ngspice.mjs', '/ngspice/ngspice.wasm');
  const ldrOhms = (lux) => 10_000 * Math.pow(Math.max(lux, 0.01) / 10, -0.7);
  const ldrVolts = (lux) => 3.3 * ldrOhms(lux) / (10_000 + ldrOhms(lux));
  // The model the host's exchange receives, from the manifest's own config.
  const model = (circuit, inputs, outputs, outputNodes) => ({
    id: 'ldr', model: './ldr.cir', stepNs: 100_000, transientBudgetStepSeconds: 10e-6,
    inputs, outputs, netlist: circuit,
    inputSources: {
      supply: { element: 'Vsup', scale: 3.3, offset: 0 },
      lux: { element: 'Vlux', scale: 1, offset: 0, default_value: 100 },
    },
    outputNodes,
  });
  const cases = [];
  const run = async (name, body) => {
    try {
      const detail = await body();
      cases.push({ name, pass: true, ...detail });
    } catch (error) {
      cases.push({ name, pass: false, error: String(error?.message ?? error) });
    }
  };
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  const lastVolts = (sim, cursor = 0) => {
    const batch = sim.analog_trace_snapshot(cursor);
    const index = batch.channels.findIndex((c) => c.name === 'ldr.v_out');
    check(index >= 0, `no ldr.v_out channel: ${JSON.stringify(batch.channels)}`);
    const last = batch.samples[batch.samples.length - 1];
    return { volts: last.values[index], samples: batch.samples.length };
  };

  const machineModel = (circuit) => model(circuit,
    { supply: 'board.gpio.pa5', lux: 'ui.ldr.lux' },
    { v_out: 'board.analog.pa0_volts' },
    { v_out: { node: 'out', scale: 1, offset: 0 } });

  // Machine: F401 blinky drives PA5 (divider supply) in lockstep with ngspice.
  await run('machine: firmware-switched LDR, bright then dark', async () => {
    const exchange = await host.createBrowserNgspiceExchange([machineModel(netlist)], factory);
    const sim = wasm.WasmSimulator.new_from_config_with_ngspice_callback(
      systemYaml, chipYaml, new Uint8Array(firmware), null, exchange.exchange);
    try {
      sim.set_cosim_signal('ui.ldr.lux', 100);
      // 84 MHz core, 100 us boundary = 8400 cycles; 20 ms lets blink drive PA5
      // high and the divider settle (>= 30 time constants).
      for (let i = 0; i < 200; i += 1) sim.step_batch(8_400);
      const bright = lastVolts(sim);
      sim.set_cosim_signal('ui.ldr.lux', 1);
      for (let i = 0; i < 100; i += 1) sim.step_batch(8_400);
      const dark = lastVolts(sim);
      check(Math.abs(bright.volts - ldrVolts(100)) < 1e-4, `bright ${bright.volts} V vs ${ldrVolts(100)} V`);
      check(Math.abs(dark.volts - ldrVolts(1)) < 1e-4, `dark ${dark.volts} V vs ${ldrVolts(1)} V`);
      return { bright_v: bright.volts, bright_model_v: ldrVolts(100), dark_v: dark.volts,
        dark_model_v: ldrVolts(1), scope_samples: dark.samples };
    } finally { sim.free(); exchange.dispose(); }
  });

  // Negative control: a source the netlist does not declare fails the run.
  await run('machine: missing source element is an error', async () => {
    const broken = machineModel(netlist);
    broken.inputSources.supply.element = 'Vmissing';
    let exchange;
    try {
      exchange = await host.createBrowserNgspiceExchange([broken], factory);
      const sim = wasm.WasmSimulator.new_from_config_with_ngspice_callback(
        systemYaml, chipYaml, new Uint8Array(firmware), null, exchange.exchange);
      try { for (let i = 0; i < 20; i += 1) sim.step_batch(8_400); } finally { sim.free(); }
    } catch (error) {
      const message = String(error?.message ?? error);
      check(/vmissing/i.test(message), `the error must name the missing element: ${message}`);
      return { refused: message };
    } finally { exchange?.dispose(); }
    throw new Error('a missing source element must fail');
  });

  // Snapshot refusal: an external solver's state is not in the machine.
  await run('machine: runtime snapshot is refused', async () => {
    const exchange = await host.createBrowserNgspiceExchange([machineModel(netlist)], factory);
    const sim = wasm.WasmSimulator.new_from_config_with_ngspice_callback(
      systemYaml, chipYaml, new Uint8Array(firmware), null, exchange.exchange);
    try {
      sim.step_batch(8_400);
      try { sim.take_runtime_snapshot(); } catch (error) {
        const message = String(error?.message ?? error);
        check(message.includes('ngspice'), message);
        return { refused: message };
      }
      throw new Error('take_runtime_snapshot must be refused');
    } finally { sim.free(); exchange.dispose(); }
  });

  // Flex: the same manifest semantics with flat signal names, host-stepped.
  await run('flex: LDR divider, unpowered then bright then dark', async () => {
    // The app's own Flex model preparation reads the manifest's config.
    const exchange = await host.createBrowserNgspiceExchange(
      host.prepareBrowserFlexNgspiceModels(flexYaml, { './ldr.cir': netlist }), factory);
    let flex;
    try {
      flex = wasm.WasmFlexRuntime.new_from_yaml_with_ngspice_callback(flexYaml, exchange.exchange);
      const port = (name) => flex.port_id(name);
      flex.advance_ns(1_000_000);
      check(Math.abs(flex.read(port('ldr_volts'))) < 1e-9, 'unpowered divider moved');
      flex.write(port('light'), 100);
      flex.write(port('enable'), 1);
      flex.advance_ns(10_000_000);
      const bright = flex.read(port('ldr_volts'));
      flex.write(port('light'), 1);
      flex.advance_ns(10_000_000);
      const dark = flex.read(port('ldr_volts'));
      check(Math.abs(bright - ldrVolts(100)) < 1e-4, `bright ${bright} V`);
      check(Math.abs(dark - ldrVolts(1)) < 1e-4, `dark ${dark} V`);
      check(flex.read(port('dark')) === 1, 'dark threshold output did not flip');
      return { bright_v: bright, dark_v: dark };
    } finally { flex?.free(); exchange.dispose(); }
  });

  // Flex test scripts: the committed portable firmware (firmware.rs) runs in
  // the package's wasmi runner against the ngspice RC of system-ngspice.yaml,
  // through the app's own Flex model preparation and exchange.
  const [rcYaml, rcNetlist, enableScript, disabledScript, rcFirmware] = await Promise.all([
    fetch('/mixed/system-ngspice.yaml').then((r) => r.text()),
    fetch('/mixed/rc.cir').then((r) => r.text()),
    fetch('/mixed/ngspice-enable.yaml').then((r) => r.text()),
    fetch('/mixed/ngspice-disabled.yaml').then((r) => r.text()),
    fetch('/mixed/firmware.wasm').then((r) => r.arrayBuffer()),
  ]);
  const rcModels = () => host.prepareBrowserFlexNgspiceModels(rcYaml, { './rc.cir': rcNetlist });
  // rc.cir: tau = 10 kOhm * 100 nF. Under `input_sampling: previous_boundary`
  // the drive set at 1 us is latched at the 25 us boundary and applied from it.
  const RC_TAU_NS = 1_000_000;
  const RC_DRIVE_FROM_NS = 25_000;
  const rcVolts = (timeNs) => 3.3 * (1 - Math.exp(-Math.max(timeNs - RC_DRIVE_FROM_NS, 0) / RC_TAU_NS));
  const RC_TOLERANCE_V = 1e-5;
  const testRun = (name, script, manifest, firmware, callback,
    channelName = 'rc.v_out', captureDetails = false) => {
    const run = wasm.WasmFlexTestRun.run_with_ngspice_callback(
      name, script, manifest, new Uint8Array(firmware), callback);
    try {
      const result = JSON.parse(run.result_json());
      let samples = null;
      if (result.stop_reason !== 'config_error') {
        const runtime = run.take_runtime();
        try {
          const batch = runtime.analog_trace_snapshot(0);
          const index = batch.channels.findIndex((channel) => channel.name === channelName);
          check(index >= 0 && batch.channels[index].unit === 'V',
            `scope channel ${channelName}: ${JSON.stringify(batch.channels)}`);
          samples = batch.samples.map((s) => ({ time_ns: Number(s.time_ns), v: s.values[index] }));
          if (captureDetails) {
            return {
              result,
              samples,
              batch,
              digital: runtime.digital_trace_snapshot(0),
              report: runtime.run_report(),
            };
          }
          return { result, samples };
        } finally { runtime.free(); }
      }
      return { result, samples };
    } finally { run.free(); }
  };
  const failedAssertions = (result) => result.assertions.filter((a) => !a.passed);
  const sampleAt = (samples, timeNs) => samples.find((s) => s.time_ns === timeNs)?.v;

  await run('flex test: firmware enables the ngspice RC', async () => {
    const exchange = await host.createBrowserNgspiceExchange(rcModels(), factory);
    try {
      const { result, samples } = testRun(
        'ngspice-enable.yaml', enableScript, rcYaml, rcFirmware, exchange.exchange);
      check(result.status === 'pass' && result.stop_reason === 'completed',
        `${result.status}/${result.stop_reason}: ${result.message} ${JSON.stringify(failedAssertions(result))}`);
      let worst = 0;
      for (const s of samples) worst = Math.max(worst, Math.abs(s.v - rcVolts(s.time_ns)));
      check(worst < RC_TOLERANCE_V, `ngspice RC departs from the analytic curve by ${worst} V`);
      return { entry_result: result.entry_result, time_ns: result.time_ns,
        v_at_1000000_ns: sampleAt(samples, 1_000_000), v_at_1025000_ns: sampleAt(samples, 1_025_000),
        analytic_v_at_1025000_ns: rcVolts(1_025_000), worst_abs_error_v: worst,
        scope_samples: samples.length, assertions: result.assertions.length };
    } finally { exchange.dispose(); }
  });

  await run('flex test: wrong firmware command leaves the ngspice RC low', async () => {
    const exchange = await host.createBrowserNgspiceExchange(rcModels(), factory);
    try {
      const { result, samples } = testRun(
        'ngspice-disabled.yaml', disabledScript, rcYaml, rcFirmware, exchange.exchange);
      check(result.status === 'pass' && result.stop_reason === 'completed',
        `${result.status}/${result.stop_reason}: ${result.message} ${JSON.stringify(failedAssertions(result))}`);
      check(result.entry_result === 0, `entry_result ${result.entry_result}`);
      const peak = Math.max(...samples.map((s) => Math.abs(s.v)));
      check(peak === 0, `the undriven circuit moved to ${peak} V`);
      return { entry_result: result.entry_result, time_ns: result.time_ns, peak_v: peak,
        scope_samples: samples.length };
    } finally { exchange.dispose(); }
  });

  // Controls: a callback that throws, or answers without a declared output,
  // mid-run. Each must end the script as a model error, never as a pass, at
  // the last boundary that completed (39 x 25 us) and with its own reason.
  const FAULT_TIME_NS = 950_000;
  const faulty = (label, answer, reason) => run(`flex test: ${label} is a model error`, async () => {
    const exchange = await host.createBrowserNgspiceExchange(rcModels(), factory);
    try {
      let calls = 0;
      const callback = (request) => {
        calls += 1;
        // Exchange 1 is the (0, 0) priming step; fail at the 40th boundary.
        return calls < 40 ? exchange.exchange(request) : answer(request);
      };
      const { result } = testRun(
        'ngspice-enable.yaml', enableScript, rcYaml, rcFirmware, callback);
      check(result.status === 'error' && result.stop_reason === 'model_error',
        `${result.status}/${result.stop_reason}: ${result.message}`);
      check(result.time_ns === FAULT_TIME_NS, `stopped at ${result.time_ns} ns, expected ${FAULT_TIME_NS} ns`);
      check(String(result.message).includes(reason), `message must contain '${reason}': ${result.message}`);
      return { stop_reason: result.stop_reason, message: result.message, time_ns: result.time_ns };
    } finally { exchange.dispose(); }
  });
  await faulty('a throwing ngspice callback', () => { throw new Error('solver host crashed'); },
    'solver host crashed');
  await faulty('a callback response without v_out', () => '{}', 'omitted output v_out');
  await faulty('a callback response that is not JSON', () => 'not json', 'returned invalid JSON');

  await run('flex test: a non-function callback is refused', async () => {
    try {
      wasm.WasmFlexTestRun.run_with_ngspice_callback(
        'ngspice-enable.yaml', enableScript, rcYaml, new Uint8Array(rcFirmware), 'exchange');
    } catch (error) {
      const message = String(error?.message ?? error);
      check(message.includes('must be a function'), message);
      return { refused: message };
    }
    throw new Error('a non-function callback must be refused');
  });

  await run('flex test: plain run() refuses the ngspice manifest', async () => {
    const run = wasm.WasmFlexTestRun.run('ngspice-enable.yaml', enableScript, rcYaml, new Uint8Array(rcFirmware));
    try {
      const result = JSON.parse(run.result_json());
      check(result.status === 'error' && result.stop_reason === 'config_error', `${result.status}/${result.stop_reason}`);
      check(result.message.includes('needs an ngspice callback'), result.message);
      return { stop_reason: result.stop_reason, message: result.message };
    } finally { run.free(); }
  });

  // The exact source-pinned HeyPCB project and portable behavior firmware,
  // exercising the ngspice adapter through the same browser Flex test API.
  const [heypcbYaml, heypcbNetlist, heypcbFirmware, brightScript, dimScript,
    sourceMapText, firmwareSource, browserReceiptText] = await Promise.all([
    fetch('/heypcb-ldr/system-ngspice.yaml').then((r) => r.text()),
    fetch('/heypcb-ldr/ldr.cir').then((r) => r.text()),
    fetch('/heypcb-ldr/firmware.wasm').then((r) => r.arrayBuffer()),
    fetch('/heypcb-ldr/ngspice-bright.yaml').then((r) => r.text()),
    fetch('/heypcb-ldr/ngspice-dim.yaml').then((r) => r.text()),
    fetch('/heypcb-ldr/source-map.json').then((r) => r.text()),
    fetch('/heypcb-ldr/firmware.rs').then((r) => r.text()),
    fetch('/heypcb-ldr/browser.json').then((r) => r.text()),
  ]);
  const sourceMap = JSON.parse(sourceMapText);
  const browserReceipt = JSON.parse(browserReceiptText);
  const source = sourceMap.source;
  const sourceIdentity = {
    repository: source.repository,
    revision: source.revision,
    asset: source.asset,
    board_revision: source.board_revision,
    license_spdx: source.license_spdx,
    files: source.files,
  };
  const sha256 = async (data) => [...new Uint8Array(await crypto.subtle.digest(
    'SHA-256', typeof data === 'string' ? new TextEncoder().encode(data) : data))]
    .map((byte) => byte.toString(16).padStart(2, '0')).join('');
  const inputDigests = {
    manifest: await sha256(heypcbYaml),
    netlist: await sha256(heypcbNetlist),
    firmware: await sha256(heypcbFirmware),
    firmware_source: await sha256(firmwareSource),
    source_map: await sha256(sourceMapText),
  };
  check(sourceIdentity.repository === 'HeyPCB/esp32-sensors-1'
    && sourceIdentity.revision === '57a2122a19ac1aa9c092e43fb5b123c461508752'
    && sourceIdentity.asset === 'boards/ldr/kicad/esp32_ldr'
    && sourceIdentity.board_revision === 'A'
    && sourceIdentity.license_spdx === 'CERN-OHL-S-2.0',
  `unexpected source pin: ${JSON.stringify(sourceIdentity)}`);
  check(inputDigests.firmware === browserReceipt.firmware_sha256
    && inputDigests.firmware_source === browserReceipt.firmware_source_sha256
    && inputDigests.source_map === browserReceipt.source_map_sha256,
  `portable firmware or source-map bytes changed: ${JSON.stringify(inputDigests)}`);

  const runHeypcbLdr = async (name, script, expectedEntry, expectedEdges, sampleAtNs, sampleRange) => {
    const models = host.prepareBrowserFlexNgspiceModels(heypcbYaml, { './ldr.cir': heypcbNetlist });
    const exchange = await host.createBrowserNgspiceExchange(models, factory);
    try {
      const captured = testRun(name, script, heypcbYaml, heypcbFirmware, exchange.exchange,
        'source-sns-an1.voltage', true);
      const { result, samples, batch, digital, report } = captured;
      check(result.status === 'pass' && result.stop_reason === 'completed',
        `${name}: ${result.status}/${result.stop_reason}: ${result.message}`);
      check(result.entry_result === expectedEntry,
        `${name}: firmware returned ${result.entry_result}, expected ${expectedEntry}`);
      check(batch.time_unit === 'ns' && batch.dropped === 0 && samples.length === 81
        && batch.channels[0].port_names.includes('sensor_v'),
      `${name}: incomplete named oscilloscope trace: ${JSON.stringify(batch)}`);
      const sample = samples.find((entry) => entry.time_ns === sampleAtNs)?.v;
      check(sample !== undefined && sample >= sampleRange[0] && sample <= sampleRange[1],
        `${name}: SNS_AN1 at ${sampleAtNs} ns is ${sample} V`);
      check(digital.time_unit === 'ns' && digital.dropped === 0
        && digital.edges.length === expectedEdges
        && digital.channels.some((channel) => channel.port_names.includes('user_led_drive'))
        && digital.channels.some((channel) => channel.port_names.includes('user_led_state')),
      `${name}: incomplete named logic-analyzer trace: ${JSON.stringify(digital)}`);
      check(report.execution_path === 'flex_portable_wasm' && report.target_cpu_emulated === false
        && report.execution_scope.original_target_firmware_executed === false
        && report.execution_scope.detailed_target_execution_selected === false
        && report.execution_scope.chip_parity === 'not_established'
        && report.scope_notes_complete && report.manifest_sha256 === inputDigests.manifest
        && report.assumptions.some((note) => note.includes('GL5528 row'))
        && report.unsupported_checks.some((note) => note.includes('Original ESP32-S3 firmware execution'))
        && report.unsupported_checks.some((note) => note.includes('Full-board')),
      `${name}: run report lost model scope: ${JSON.stringify(report.execution_scope)}`);
      check(['lux', 'sensor_v', 'user_led_drive', 'user_led_state']
        .every((portName) => report.ports.some((port) => port.name === portName)),
      `${name}: run report lost named ports: ${JSON.stringify(report.ports)}`);
      const model = report.models.find((entry) => entry.id === 'source-sns-an1');
      check(model?.adapter === 'ngspice' && model.source?.kind === 'spice_netlist'
        && model.source.origin === 'file' && model.source.path === './ldr.cir'
        && model.source.bytes === new TextEncoder().encode(heypcbNetlist).length
        && model.source.sha256 === inputDigests.netlist
        && model.inputs.some((input) => input.name === 'lux' && input.signal === 'environment.lux')
        && model.outputs.some((output) => output.name === 'voltage' && output.signal === 'source.U3.GPIO1'),
      `${name}: run report did not pin the source deck: ${JSON.stringify(model)}`);
      return {
        source: sourceIdentity,
        firmware: 'portable_behavior_wasm',
        input_sha256: inputDigests,
        execution_scope: report.execution_scope,
        ports: report.ports,
        model: { adapter: model.adapter, id: model.id, source: model.source },
        result: { status: result.status, entry_result: result.entry_result, time_ns: result.time_ns },
        oscilloscope: {
          time_unit: batch.time_unit,
          channels: batch.channels,
          dropped: batch.dropped,
          sample_count: samples.length,
          sample_at_ns: sampleAtNs,
          sample_v: sample,
        },
        logic_analyzer: {
          time_unit: digital.time_unit,
          channels: digital.channels,
          dropped: digital.dropped,
          edge_count: digital.edges.length,
          edge_times_ns: digital.edges.map((edge) => Number(edge.time_ns)),
        },
        scope_notes: { assumptions: report.assumptions, unsupported_checks: report.unsupported_checks },
      };
    } finally { exchange.dispose(); }
  };

  await run('HeyPCB ESP32-S3 LDR: source-pinned ngspice bright behavior', async () =>
    runHeypcbLdr('ngspice-bright.yaml', brightScript, 2974, 4, 350_000, [2.036, 2.038]));
  await run('HeyPCB ESP32-S3 LDR: ngspice dim negative control', async () =>
    runHeypcbLdr('ngspice-dim.yaml', dimScript, 597, 0, 350_000, [1.069, 1.071]));
  return { cases };
}
