// Evaluate receipt/oracle.json against a local `labwired test` run and compose
// the run/verify receipts. Uses the SAME shared evaluator the MCP
// labwired_verify tool uses (@labwired/board-config evaluateOracle +
// classifyModelVerification); the evidence is fed in the same seams the hosted
// builder feeds it:
//
//   * `serial`      <- uart.log                       (builder: result.serial)
//   * `stopReason`  <- result.json stop_reason        (builder: RunArtifact)
//   * `peripherals` <- inspect.devices[].artifacts    (builder: foldDeviceArtifacts
//                        per board-config/src/oracle.ts comments; each artifact
//                        carries the device's `device_type` verbatim)
//   * `gpio`        <- result.json logic_edges        (builder:
//                        services/labwired-builder/src/run.ts parseLogicEdges)
//
// Usage: node receipt/local-oracle.mjs compose
//   env RUN_DIR   (default <example>/receipt/run)  main run artifacts
//   env RUN2_DIR  (optional)                       second run for determinism
//   env NEG_DIR   (optional)                       negative-control run (no stimuli)
//   env LABWIRED_REPO_ROOT (default ../../../.. from this file)
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const EXAMPLE = resolve(HERE, '..');
const ROOT = process.env.LABWIRED_REPO_ROOT ?? resolve(EXAMPLE, '../../..');
const RUN_DIR = process.env.RUN_DIR ?? join(HERE, 'run');
const RUN2_DIR = process.env.RUN2_DIR;
const NEG_DIR = process.env.NEG_DIR;

const { evaluateOracle, classifyModelVerification } = await import(
  join(ROOT, 'packages/board-config/dist/index.mjs')
);

const sha = (b) => createHash('sha256').update(b).digest('hex');
const readJson = (p) => JSON.parse(readFileSync(p, 'utf-8'));

/** Mirror of services/labwired-builder/src/run.ts parseLogicEdges (lines 563-584). */
function parseLogicEdges(logicEdges) {
  if (!logicEdges || typeof logicEdges !== 'object') return undefined;
  const le = logicEdges;
  if (!Array.isArray(le.channels) || le.channels.length === 0) return undefined;
  return le.channels.map((c) => {
    const transitions = Array.isArray(c.transitions) ? c.transitions : [];
    const last = transitions.length > 0 ? transitions[transitions.length - 1].value : c.initial;
    const state = {
      pin: `${c.peripheral}:${c.pin}`,
      transitions,
      initial: c.initial ?? null,
      toggled: transitions.length > 0,
      nowCycle: le.now_cycle,
      dropped: le.dropped,
    };
    if (last === 0 || last === 1) state.value = last;
    return state;
  });
}

/** Fold result.json's inspect.devices[].artifacts into OraclePeripheralInspect[]
 *  (the hosted builder's foldDeviceArtifacts seam; oracle.ts reads
 *  artifact.device_type to match a `display: {panel}` clause). */
function foldDevices(inspect) {
  const devices = Array.isArray(inspect?.devices) ? inspect.devices : [];
  return devices.map((d) => ({
    name: d.id,
    kind: d.device_type,
    artifacts: (d.artifacts ?? []).map((a) => ({ ...a, device_type: d.device_type })),
  }));
}

function oracleRunResult(result, uart) {
  const gpio = parseLogicEdges(result.logic_edges);
  return {
    status: result.status,
    stopReason: result.stop_reason,
    serial: uart,
    peripherals: foldDevices(result.inspect),
    ...(gpio ? { gpio } : {}),
  };
}

/** Engine-stated approximations (fidelity[]) as `approximation` gaps: named
 *  modelling simplifications, NOT coverage holes (model-verification.ts). */
function approximationGaps(result) {
  return (result.fidelity ?? []).map((f) => ({
    code: 'approximation',
    scope: 'model',
    detail: `${f.kind}: ${f.detail ?? ''}`.trim(),
  }));
}

function loadRun(dir) {
  const result = readJson(join(dir, 'result.json'));
  const uart = readFileSync(join(dir, 'uart.log'));
  return { result, uart, uartSha: sha(uart) };
}

function stimulusRows(result) {
  return result.stimuli.map((s) => ({
    component: s.component,
    channel: s.channel,
    value: s.value,
    trigger: s.trigger,
    outcome: s.outcome,
    at_cycle: s.at_cycle,
  }));
}

const oracle = readJson(join(HERE, 'oracle.json'));

const main = loadRun(RUN_DIR);
const mainRunResult = oracleRunResult(main.result, main.uart.toString('utf-8'));
const mainVerdict = evaluateOracle(oracle, mainRunResult);
const gaps = approximationGaps(main.result);
const mainClassified = classifyModelVerification({ verdict: mainVerdict, gaps });

// Determinism: second run, byte-identical transcript (and manifest digest).
let determinism = { runs: 1, note: 'RUN2_DIR not provided' };
if (RUN2_DIR) {
  const second = loadRun(RUN2_DIR);
  const m1 = readJson(join(RUN_DIR, 'run-manifest.json'));
  const m2 = readJson(join(RUN2_DIR, 'run-manifest.json'));
  determinism = {
    runs: 2,
    uart_sha256: [main.uartSha, second.uartSha],
    uart_byte_identical: main.uartSha === second.uartSha,
    run_manifest_digest: [m1.digest, m2.digest],
    run_manifest_digest_identical: m1.digest === m2.digest,
  };
}

// Negative control: same oracle on a run cut at 12.5M cycles (before the 15M
// stimuli), i.e. full boot state only. The temperature/lux/BME post-stimulus
// clauses must be contradicted — proving the oracle discriminates.
let negativeControl;
if (NEG_DIR) {
  const neg = loadRun(NEG_DIR);
  const negVerdict = evaluateOracle(oracle, oracleRunResult(neg.result, neg.uart.toString('utf-8')));
  negativeControl = {
    command:
      'labwired test --script io-smoke.yaml --firmware firmware/.pio/build/blackpill_f411ce/firmware.elf ' +
      `--system system.yaml --max-steps 12500000 --watch-gpio gpioa:5 --output-dir ${NEG_DIR}`,
    note: '--max-steps 12,500,000 cuts the run before the 15,000,000-cycle stimuli: full boot state, no stimulus',
    verdict: { passed: negVerdict.passed, status: classifyModelVerification({ verdict: negVerdict }).status, clauses: negVerdict.clauses },
    uart_sha256: neg.uartSha,
  };
}

// MCP stdio attempt (archived raw capture; see receipt/mcp-attempt.mjs).
let stdioAttempt;
try {
  stdioAttempt = readJson(join(HERE, 'mcp-stdio-attempt.json'));
} catch {
  stdioAttempt = { archived: false };
}

const manifest = readJson(join(RUN_DIR, 'run-manifest.json'));
const engineBinary = process.env.ENGINE_BIN ?? join(ROOT, 'core/target/release/labwired');

const runReceipt = {
  schema_version: '1.0',
  path: 'local-engine',
  engine: { binary: engineBinary, version: manifest.engine_version },
  command:
    `${engineBinary} test --script io-smoke.yaml ` +
    '--firmware firmware/.pio/build/blackpill_f411ce/firmware.elf --system system.yaml ' +
    '--output-dir receipt/run --run-manifest --watch-gpio gpioa:5',
  inputs: {
    firmware: { path: 'firmware/.pio/build/blackpill_f411ce/firmware.elf', sha256: manifest.firmware.sha256 },
    system: { path: 'system.yaml', sha256: manifest.configs.find((c) => c.path.endsWith('system.yaml')).sha256 },
    script: { path: 'io-smoke.yaml', sha256: manifest.configs.find((c) => c.path.endsWith('io-smoke.yaml')).sha256 },
  },
  run: {
    status: main.result.status,
    stop_reason: main.result.stop_reason,
    steps_executed: main.result.steps_executed,
    cycles: main.result.cycles,
    instructions: main.result.instructions,
  },
  assertions: {
    passed: main.result.assertions.filter((a) => a.passed).length,
    total: main.result.assertions.length,
  },
  stimuli: stimulusRows(main.result),
  serial: {
    path: 'receipt/run/uart.log',
    sha256: main.uartSha,
    bytes: main.uart.length,
    lines: main.uart.toString('utf-8').trimEnd().split('\n').length,
  },
  logic_edges: main.result.logic_edges
    ? {
        watch: 'gpioa:5',
        now_cycle: main.result.logic_edges.now_cycle,
        dropped: main.result.logic_edges.dropped,
        channels: main.result.logic_edges.channels.map((c) => ({
          channel: c.channel,
          peripheral: c.peripheral,
          pin: c.pin,
          initial: c.initial,
          initial_state: c.initial_state ?? null,
          gaps: c.gaps,
          transitions: c.transitions,
        })),
        gpio_pins: main.result.gpio_pins ?? null,
      }
    : null,
  manifest: { path: 'receipt/run/run-manifest.json', digest: manifest.digest, nondeterminism: manifest.nondeterminism },
  determinism,
  artifacts: {
    result_json: 'receipt/run/result.json',
    uart_log: 'receipt/run/uart.log',
    run_manifest: 'receipt/run/run-manifest.json',
    junit: 'receipt/run/junit.xml',
  },
  mcp_stdio_run_attempt: stdioAttempt.labwired_run ?? null,
};

const verifyReceipt = {
  schema_version: '1.0',
  path: {
    evaluation: 'local engine run + @labwired/board-config evaluateOracle/classifyModelVerification (the shared evaluator)',
    run: 'labwired test --script io-smoke.yaml ... --watch-gpio gpioa:5 (receipt/run)',
    evidence_feed: 'uart.log -> serial; result.json inspect.devices -> peripherals (device_type folded per the builder seam); result.json logic_edges -> gpio (parseLogicEdges mirror)',
  },
  oracle,
  verdict: {
    status: mainClassified.status,
    proven: mainClassified.proven,
    clauses: mainVerdict.clauses,
    gaps: mainClassified.gaps,
    engine_approximations: gaps,
  },
  evidence: {
    serial: { source: 'receipt/run/uart.log', sha256: main.uartSha },
    display:
      main.result.inspect.devices.find((d) => d.id === 'oled1')?.artifacts?.[0] ?? null,
    gpio:
      main.result.logic_edges?.channels.map((c) => ({
        pin: `${c.peripheral}:${c.pin}`,
        transitions: c.transitions,
        dropped: c.dropped,
        gpio_pins: main.result.gpio_pins ?? null,
      })) ?? null,
  },
  stimuli: stimulusRows(main.result),
  negative_control: negativeControl ?? null,
  stdio_attempt: stdioAttempt.labwired_verify ?? null,
  hosted: stdioAttempt.hosted ?? null,
  unclaimed: [
    {
      clause: 'gpio_duty (PWM duty on PA5)',
      observable_on_this_path: false,
      why:
        'the logic capture on gpioa:5 recorded a single drive-enable edge (z->1 @ cycle 965,643) and a flat high tail; TIM2 compare toggles of the AF pad do not appear in the tap, so a duty computed from one edge + tail (1.0) would be an artifact, not PWM evidence',
    },
    {
      clause: 'serial/register/gpio via MCP stdio labwired_verify',
      observable_on_this_path: false,
      why:
        'stdio verify routes serial/register/gpio clauses through the hosted builder and fails without LABWIRED_COMPILE_URL (packages/mcp/src/handlers/verify.ts needBuilder; packages/mcp/src/cli.ts runBuildViaBuilder: "No builder URL configured")',
    },
    {
      clause: 'display via MCP stdio labwired_verify',
      observable_on_this_path: false,
      why:
        'the stdio device path runs `labwired snapshot capture --profile arduino-esp32` and decodes only ssd1680/uc8151d e-paper panels (packages/mcp/src/cli.ts panel-state regex); the SSD1306 OLED is not observable there (archived result: painted=false, panel=null)',
    },
    {
      clause: 'hosted labwired_run / labwired_verify',
      observable_on_this_path: false,
      why: 'hosted not attempted: no token in this environment',
    },
  ],
};

writeFileSync(join(HERE, 'run.json'), JSON.stringify(runReceipt, null, 2) + '\n');
writeFileSync(join(HERE, 'verify.json'), JSON.stringify(verifyReceipt, null, 2) + '\n');
console.log(JSON.stringify({
  status: mainClassified.status,
  proven: mainClassified.proven,
  clauses: mainVerdict.clauses.map((c) => `${c.passed ? 'PASS' : 'FAIL'} ${c.kind}: ${c.detail}`),
  determinism,
  written: ['receipt/run.json', 'receipt/verify.json'],
}, null, 2));
