// SPDX-License-Identifier: MIT
// Shared assertions for Node and a real browser. The firmware is portable
// WebAssembly; the imported CH32V003 HEX is not executed by this fixture.
export function runPomodoroPd6Checks(WasmFlexRuntime, system, firmwareBytes, expected) {
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  const canonical = (v) => typeof v === 'bigint' ? Number(v) : Array.isArray(v) ? v.map(canonical)
    : v !== null && typeof v === 'object'
      ? Object.fromEntries(Object.keys(v).sort().map((key) => [key, canonical(v[key])])) : v;
  const equal = (got, wanted, message) => check(JSON.stringify(canonical(got)) === JSON.stringify(canonical(wanted)), message);
  const firmware = new WebAssembly.Module(firmwareBytes);
  equal(WebAssembly.Module.imports(firmware).map(({ module, name }) => `${module}.${name}`).sort(),
    ['labwired.advance_ns', 'labwired.read', 'labwired.write'], 'unexpected portable firmware imports');

  const trace = (runtime) => {
    const batch = canonical(runtime.digital_trace_snapshot(0n));
    const signalById = new Map(batch.channels.map((channel) => [channel.id, channel.signal]));
    return {
      time_unit: batch.time_unit,
      dropped: batch.dropped,
      channels: batch.channels.map(({ signal, initial }) => ({ signal, initial }))
        .sort((a, b) => a.signal.localeCompare(b.signal)),
      edges: batch.edges.map((edge) => ({
        signal: signalById.get(edge.ch), time_ns: edge.time_ns, value: edge.value,
      })).sort((a, b) => a.time_ns - b.time_ns || a.signal.localeCompare(b.signal)),
    };
  };

  const run = (mode) => {
    const runtime = WasmFlexRuntime.new_from_yaml(system);
    try {
      const instance = new WebAssembly.Instance(firmware, { labwired: {
        read: (port) => runtime.read(port),
        write: (port, value) => runtime.write(port, value),
        advance_ns: (delta) => runtime.advance_ns(delta),
      } });
      const result = instance.exports.run(runtime.port_id('u1_pd6'), runtime.port_id('d1_state'), runtime.port_id('d2_state'), mode);
      return { result, time_ns: Number(runtime.time_ns()), trace: trace(runtime) };
    } finally {
      runtime.free();
    }
  };

  const positive = run(0);
  equal(positive, expected.positive, 'portable positive run differs from the native model reference');
  check(positive.result === 1, 'positive run did not observe both D1/D2 logic outputs');
  check(positive.trace.dropped === 0, 'positive digital trace lost events');
  const negative = run(1);
  equal(negative, expected.negative, 'negative control differs from the native model reference');
  check(negative.result === 0 && negative.trace.edges.length === 0, 'negative control activated an LED logic output');

  return {
    status: 'passed',
    runtime: 'compiled-wasm',
    firmware_bytes: firmwareBytes.length,
    cpu_emulation: false,
    source_board: 'HeyPCB Pomodoro Pal / ch32v003f4p6',
    observable: 'source.U1.PD6 drives D1/D2 logic-state outputs',
    positive_edges: positive.trace.edges.length,
    negative_edges: negative.trace.edges.length,
    checks: ['portable-firmware-execution', 'native-model-equality', 'source-pin-mapping', 'positive-output', 'negative-control'],
  };
}
