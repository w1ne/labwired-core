// SPDX-License-Identifier: MIT
// Shared assertions for Node and an actual browser. No host or DOM APIs.
export function runPortableChecks(WasmFlexRuntime, system, firmwareBytes, expected) {
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  const canonical = (v) => typeof v === 'bigint' ? Number(v) : Array.isArray(v) ? v.map(canonical)
    : v !== null && typeof v === 'object'
      ? Object.fromEntries(Object.keys(v).sort().map((key) => [key, canonical(v[key])])) : v;
  const equal = (got, wanted, message) => check(JSON.stringify(canonical(got)) === JSON.stringify(canonical(wanted)), message);
  const throws = (fn, message) => {
    let didThrow = false;
    try { fn(); } catch { didThrow = true; }
    check(didThrow, message);
  };
  const firmware = new WebAssembly.Module(firmwareBytes);
  equal(WebAssembly.Module.imports(firmware).map(({ module, name }) => `${module}.${name}`).sort(), ['labwired.advance_ns', 'labwired.read', 'labwired.write'], 'unexpected firmware imports');
  check(expected.cases?.length === 2, 'native model evidence missing');
  const floatBits = (value) => {
    const view = new DataView(new ArrayBuffer(4));
    view.setFloat32(0, value, true);
    return view.getUint32(0, true).toString(16).padStart(8, '0');
  };
  const build = (yaml = system) => {
    const runtime = WasmFlexRuntime.new_from_yaml(yaml);
    const instance = new WebAssembly.Instance(firmware, { labwired: {
      read: (port) => runtime.read(port),
      write: (port, value) => runtime.write(port, value),
      advance_ns: (delta) => runtime.advance_ns(delta),
    } });
    return { runtime, instance };
  };
  const digital = (runtime, driven) => {
    const batch = canonical(runtime.digital_trace_snapshot(0n));
    check(batch.time_unit === 'ns' && batch.dropped === 0, 'digital trace time unit or loss');
    equal(batch.channels.map((c) => c.signal), ['control.enable', 'device.ready'], 'stable digital channel names');
    check(batch.channels.every((c) => c.initial === false), 'initial digital levels');
    equal(batch.edges, driven ? [
      { ch: batch.channels[0].id, time_ns: 0, value: 1 },
      { ch: batch.channels[1].id, time_ns: 1000, value: 1 },
    ] : [], 'digital transitions must preserve model event times');
    return batch;
  };
  const observations = (runtime, tau_us, timeConstantNs) => {
    const batch = runtime.analog_trace_snapshot(0n);
    check(batch.samples.length >= 35, 'empty/truncated analog trace');
    check(batch.samples.every((s) => {
      const analytic = 3.3 * (1 - Math.exp(-Math.max(Number(s.time_ns) - 25000, 0) / timeConstantNs));
      return Math.abs(s.values[0] - analytic) < 0.04;
    }), 'analog trace must follow the sampled RC charge curve');
    return { tau_us, time_ns: Number(runtime.time_ns()), trace: {
      channels: batch.channels.map((c) => c.name),
      rows: batch.samples.map((s) => [Number(s.time_ns), floatBits(s.values[0])]),
    }, digital: digital(runtime, true) };
  };
  const cases = [];
  for (const [index, yaml] of [system, system.replace('R1 in out 10k', 'R1 in out 20k')].entries()) {
    const { runtime, instance } = build(yaml);
    try {
      const tau = instance.exports.run(runtime.port_id('command'), runtime.port_id('voltage'), 0);
      const result = observations(runtime, tau, (index + 1) * 1_000_000);
      equal(result, expected.cases[index], `portable firmware/browser model differs from native model case ${index}: ${JSON.stringify(result)}`);
      cases.push({ resistance_ohms: index === 0 ? 10000 : 20000, tau_us: tau, samples: result.trace.rows.length, digital_edges: result.digital.edges.length });
    } finally { runtime.free(); }
  }
  const wrong = build();
  try {
    check(wrong.instance.exports.run(wrong.runtime.port_id('command'), wrong.runtime.port_id('voltage'), 1) === 0, 'incorrect firmware must time out');
    check(Number(wrong.runtime.time_ns()) === 4_000_000, 'incorrect firmware timeout must be bounded');
    digital(wrong.runtime, false);
    check(wrong.runtime.analog_trace_snapshot(0n).samples.every((s) => s.values[0] === 0), 'wrong command fabricated an analog response');
  } finally { wrong.runtime.free(); }
  const invalid = build();
  try {
    const command = invalid.runtime.port_id('command');
    const voltage = invalid.runtime.port_id('voltage');
    throws(() => invalid.instance.exports.run(0xffffffff, voltage, 0), 'invalid firmware port must propagate a host error');
    throws(() => invalid.instance.exports.run(voltage, voltage, 0), 'firmware cannot write a read-only observation');
    throws(() => invalid.runtime.read(command), 'write-only command must reject reads');
    throws(() => invalid.runtime.write(command, NaN), 'non-finite input');
    throws(() => invalid.runtime.write(command, Infinity), 'non-finite input');
  } finally { invalid.runtime.free(); }
  throws(() => build(system.replace('unit: V', 'unit: mV')), 'no implicit unit conversion');
  throws(() => build(system.replace('initial: 0', 'initial: 1')), 'analog DC source must agree with initial drive');
  throws(() => build(system.replace('name: voltage, signal: circuit.voltage', 'name: voltage, signal: missing.voltage')), 'unbound read port');
  throws(() => build(system.replace('primitive: gpio_device', 'primitive: invented_device')), 'unsupported behavior');
  return { status: 'passed', runtime: 'compiled-wasm', firmware_bytes: firmwareBytes.length, cpu_emulation: false, cases, checks: ['portable-firmware-execution', 'native-wasm-model-equality', 'analog-rc-curve', 'digital-transition-timestamps', 'yaml-only-parameter-reuse', 'wrong-command', 'invalid-port', 'access-direction', 'non-finite-input', 'unit-mismatch', 'inconsistent-initial-state', 'unbound-port', 'unsupported-behavior'] };
}
