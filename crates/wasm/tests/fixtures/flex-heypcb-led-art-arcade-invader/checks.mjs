// SPDX-License-Identifier: MIT
// Exercise the same generic test-run API and instrument snapshot in Node and Chromium.
export async function runLedArtChecks(_WasmFlexRuntime, manifest, firmwareBytes, expected, WasmFlexTestRun) {
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  check(typeof WasmFlexTestRun?.run === 'function', 'WasmFlexTestRun is required');
  const canonical = (value) => typeof value === 'bigint' ? Number(value)
    : Array.isArray(value) ? value.map(canonical)
      : value !== null && typeof value === 'object'
        ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]))
        : value;
  const equal = (got, wanted, message) => {
    check(JSON.stringify(canonical(got)) === JSON.stringify(canonical(wanted)), message);
  };
  const loadText = async (name) => {
    const url = new URL(name, import.meta.url);
    if (url.protocol === 'file:') {
      return import('node:fs').then(({ readFileSync }) => readFileSync(url, 'utf8'));
    }
    const response = await fetch(url);
    check(response.ok, `failed to fetch ${name}`);
    return response.text();
  };
  const source = expected.source;
  check(source.repository === 'HeyPCB/led-art', 'unexpected source repository');
  check(source.revision === 'b0830dcfa3f71c409e9784fa784b9c6864a6ff12', 'unexpected pinned revision');
  check(source.asset === 'boards/arcade-invader' && source.license_spdx === 'CERN-OHL-S-2.0', 'unexpected source identity or license');
  check(firmwareBytes.length > 0, 'portable firmware is empty');

  const cases = {};
  for (const name of ['positive', 'negative']) {
    const script = await loadText(`test-${name}.yaml`);
    const run = WasmFlexTestRun.run(`test-${name}.yaml`, script, manifest, firmwareBytes);
    try {
      const result = JSON.parse(run.result_json());
      const reference = expected[name];
      check(result.status === 'pass' && result.stop_reason === 'completed', `${name} test did not pass`);
      equal(result.entry_result, reference.entry_result, `${name} entry result differs from native`);
      equal(result.time_ns, reference.time_ns, `${name} duration differs from native`);
      equal(result.config.firmware_sha256, source.firmware_sha256, `${name} firmware bytes differ from native`);
      const report = result.run_report;
      check(report.execution_path === 'flex_portable_wasm' && report.target_cpu_emulated === false, `${name} is not portable Flex execution`);
      check(report.manifest_sha256 === source.manifest_sha256, `${name} manifest differs from native`);
      check(report.execution_scope.original_target_firmware_executed === false, `${name} claims original firmware execution`);
      check(report.execution_scope.detailed_target_execution_selected === false, `${name} selected detailed target execution`);
      check(report.execution_scope.full_board_coverage === 'not_established' && report.execution_scope.chip_parity === 'not_established', `${name} overstates coverage`);
      check(report.scope_notes_complete === true && report.assumptions.length > 0 && report.unsupported_checks.length > 0, `${name} omits model limits`);
      check(report.ports.some((port) => port.name === 'led_data' && port.signal === 'source.U1.PC6'), `${name} named port does not map to imported PC6`);
      check(report.sinks.some((sink) => sink.id === 'strip_d1' && sink.model_type === 'neopixel' && sink.pins.DIN === 'source.U1.PC6'), `${name} generic sink is not bound to imported PC6`);
      equal(result.traces.digital_edges, reference.digital_edges, `${name} edge count differs from native`);
      equal(result.traces.digital_dropped, 0, `${name} dropped trace edges`);
      equal(result.traces.time_unit, 'ns', `${name} trace time unit`);
      const device = result.devices.find(({ id }) => id === 'strip_d1');
      check(device?.state.pixels_decoded === reference.pixels_decoded, `${name} decoded pixel count differs from native`);
      check(device.state.lit_pixels === reference.lit_pixels, `${name} lit pixel count differs from native`);
      const frame = device.state.frame_history[0]?.bytes;
      check(frame?.length === reference.frame_bytes, `${name} frame length differs from native`);
      equal(frame.slice(0, reference.frame_prefix.length), reference.frame_prefix, `${name} GRB frame prefix differs from native`);
      equal(frame.slice(6), new Array(57).fill(0), `${name} remaining pixels are not dark`);
      check(device.state.frame_history.length === 1 && device.state.frame_history_dropped === 0, `${name} frame history is incomplete`);

      const runtime = run.take_runtime();
      try {
        const batch = canonical(runtime.digital_trace_snapshot(0));
        const channelById = new Map(batch.channels.map((channel) => [channel.id, channel]));
        const edgePairs = batch.edges.map(({ time_ns, value }) => [time_ns, value]);
        const edgeDigest = await globalThis.crypto.subtle.digest(
          'SHA-256', new TextEncoder().encode(JSON.stringify(edgePairs)));
        const edgeSha256 = Array.from(new Uint8Array(edgeDigest), (byte) => byte.toString(16).padStart(2, '0')).join('');
        check(edgeSha256 === reference.edge_sha256, `${name} complete digital trace differs from native`);
        const firstEdges = batch.edges.slice(0, reference.first_edges.length).map((edge) => [edge.time_ns, edge.value]);
        equal(firstEdges, reference.first_edges, `${name} nanosecond trace differs from native`);
        check(batch.time_unit === 'ns' && batch.dropped === 0, `${name} instrument capture is incomplete`);
        check(batch.edges.length === reference.digital_edges, `${name} instrument capture edge count`);
        check(batch.channels.length === 1, `${name} instrument channel count`);
        const channel = channelById.get(batch.channels[0].id);
        check(channel.signal === 'source.U1.PC6' && channel.port_names.includes('led_data'), `${name} trace channel lost its named imported net`);
        cases[name] = {
          status: result.status,
          entry_result: result.entry_result,
          time_ns: result.time_ns,
          digital_edges: batch.edges.length,
          trace_channel: channel.signal,
          trace_port_names: channel.port_names,
          pixels_decoded: device.state.pixels_decoded,
          lit_pixels: device.state.lit_pixels,
          frame_bytes: frame.length,
        };
      } finally {
        runtime.free();
      }
    } finally {
      run.free();
    }
  }
  check(cases.positive.lit_pixels === 2 && cases.negative.lit_pixels === 0, 'positive or negative LED control failed');
  return {
    status: 'passed',
    runtime: 'WasmFlexTestRun',
    cpu_emulation: false,
    source: {
      repository: source.repository,
      revision: source.revision,
      asset: source.asset,
      license_spdx: source.license_spdx,
    },
    native_reference_equality: true,
    cases,
    checks: ['portable-test-runner', 'source-and-license-pin', 'named-imported-port', 'nanosecond-logic-analyzer',
      'neopixel-frame-decode', 'assumptions-and-unsupported-scope', 'positive-output', 'negative-control'],
  };
}
