// SPDX-License-Identifier: MIT
// The same runner is used by Node and Chromium. Firmware is portable Wasm;
// neither the AVR nor CH32 target CPU participates.
export async function runGpioReuseChecks(WasmFlexRuntime, system, firmwareBytes, expected) {
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  const canonical = (v) => typeof v === 'bigint' ? Number(v) : Array.isArray(v) ? v.map(canonical)
    : v !== null && typeof v === 'object'
      ? Object.fromEntries(Object.keys(v).sort().map((key) => [key, canonical(v[key])])) : v;
  const equal = (got, wanted, message) => check(JSON.stringify(canonical(got)) === JSON.stringify(canonical(wanted)), message);
  const firmware = new WebAssembly.Module(firmwareBytes);
  equal(WebAssembly.Module.imports(firmware).map(({ module, name }) => module + '.' + name).sort(),
    ['labwired.advance_ns', 'labwired.read', 'labwired.write'], 'unexpected portable firmware imports');

  const trace = (runtime) => {
    const batch = canonical(runtime.digital_trace_snapshot(0));
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
  const run = (system, pulses, hasSecondLed = false) => {
    const runtime = WasmFlexRuntime.new_from_yaml(system);
    try {
      const secondLedPort = hasSecondLed ? runtime.port_id('led_state_2') : 0xffffffff;
      const instance = new WebAssembly.Instance(firmware, { labwired: {
        read: (port) => runtime.read(port),
        write: (port, value) => runtime.write(port, value),
        advance_ns: (delta) => runtime.advance_ns(delta),
      } });
      const result = instance.exports.run(
        runtime.port_id('drive'), runtime.port_id('led_state'), secondLedPort, pulses,
      );
      return { result, time_ns: Number(runtime.time_ns()), trace: trace(runtime) };
    } finally {
      runtime.free();
    }
  };
  const load = async (name) => {
    const url = new URL(name, import.meta.url);
    return url.protocol === 'file:'
      ? import('node:fs').then(({ readFileSync }) => readFileSync(url, 'utf8'))
      : fetch(url).then((r) => {
          check(r.ok, 'failed to fetch ' + name);
          return r.text();
        });
  };
  const fixture = (yaml, name, expectedCase, hasSecondLed = false) => {
    const positive = run(yaml, 0, hasSecondLed);
    equal(positive, expectedCase.positive, name + ' positive run differs from the native reference');
    check(positive.result === 1, name + ' did not observe every LED pulse');
    check(positive.time_ns === 5000, name + ' positive run duration');
    const expectedEdges = hasSecondLed ? 12 : 8;
    check(positive.trace.edges.length === expectedEdges, name + ' positive edge count');
    check(positive.trace.dropped === 0 && positive.trace.time_unit === 'ns', name + ' positive trace quality');
    const negative = run(yaml, 1, hasSecondLed);
    equal(negative, expectedCase.negative, name + ' negative control differs from the native reference');
    check(negative.result === 0 && negative.trace.edges.length === 0, name + ' negative control activated the LED');
    return { positive, negative };
  };
  const pomodoro = fixture(system, 'HeyPCB Pomodoro Pal', expected.pomodoro, true);
  const arduinoUno = fixture(await load('arduino-uno.yaml'), 'Arduino Uno model', expected.arduino_uno);
  return {
    status: 'passed',
    runtime: 'compiled-wasm',
    firmware_bytes: firmwareBytes.length,
    cpu_emulation: false,
    bindings: {
      heypcb_pomodoro: {
        drive: 'source.U1.PD6',
        observables: ['source.D1.logic_state', 'source.D2.logic_state'],
        positive_edges: pomodoro.positive.trace.edges.length,
        negative_edges: pomodoro.negative.trace.edges.length,
      },
      arduino_uno_model: {
        drive: 'arduino.portb.pb5',
        observable: 'arduino.status_led.logic_state',
        workload: 'one-microsecond pulse plus two-microsecond pulse (held out from the one-pulse proof)',
        positive_edges: arduinoUno.positive.trace.edges.length,
        negative_edges: arduinoUno.negative.trace.edges.length,
      },
    },
    native_model_equality: true,
    checks: ['portable-firmware-execution', 'two-board-bindings', 'two-source-led-outputs',
      'held-out-asymmetric-pulse-workload', 'native-model-equality', 'positive-output', 'negative-control'],
  };
}
