// SPDX-License-Identifier: MIT
// Portable Wasm firmware + generic Flex runtime checks for the LDR behavior slice.
export function runHeypcbLdrChecks(WasmFlexRuntime, system, firmwareBytes, expected) {
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  const canonical = (v) => typeof v === 'bigint' ? Number(v) : Array.isArray(v) ? v.map(canonical)
    : v !== null && typeof v === 'object'
      ? Object.fromEntries(Object.keys(v).sort().map((key) => [key, canonical(v[key])])) : v;
  const equal = (got, wanted, message) => {
    const actual = JSON.stringify(canonical(got));
    const expectedJson = JSON.stringify(canonical(wanted));
    check(actual === expectedJson, `${message}; got ${actual}; expected ${expectedJson}`);
  };
  const firmware = new WebAssembly.Module(firmwareBytes);
  equal(WebAssembly.Module.imports(firmware).map(({ module, name }) => `${module}.${name}`).sort(),
    ['labwired.advance_ns', 'labwired.read', 'labwired.write'], 'unexpected portable firmware imports');

  const run = (lux) => {
    const runtime = WasmFlexRuntime.new_from_yaml(system);
    try {
      const ports = Object.fromEntries(['lux', 'sensor_mv', 'user_led_drive', 'user_led_state']
        .map((name) => [name, runtime.port_id(name)]));
      const instance = new WebAssembly.Instance(firmware, { labwired: {
        read: (port) => runtime.read(port),
        write: (port, value) => runtime.write(port, value),
        advance_ns: (delta) => runtime.advance_ns(delta),
      } });
      const adcCode = instance.exports.run(
        ports.lux, ports.sensor_mv, ports.user_led_drive, ports.user_led_state, lux,
      );
      const batch = canonical(runtime.digital_trace_snapshot(0n));
      const signalById = new Map(batch.channels.map((channel) => [channel.id, channel.signal]));
      return {
        result: adcCode,
        lux,
        sensor_mv: runtime.read(ports.sensor_mv),
        adc_code: adcCode,
        led_final: runtime.read(ports.user_led_state),
        time_ns: Number(runtime.time_ns()),
        trace: {
          time_unit: batch.time_unit,
          dropped: batch.dropped,
          channels: batch.channels.map(({ signal, initial }) => ({ signal, initial }))
            .sort((a, b) => a.signal.localeCompare(b.signal)),
          edges: batch.edges.map((edge) => ({
            signal: signalById.get(edge.ch), time_ns: edge.time_ns, value: edge.value,
          })).sort((a, b) => a.time_ns - b.time_ns || a.signal.localeCompare(b.signal)),
        },
      };
    } finally {
      runtime.free();
    }
  };

  const positive = run(100000);
  equal(positive, expected.positive, 'bright portable run differs from the native Flex model reference');
  check(positive.adc_code >= 4080 && positive.adc_code <= 4090, 'bright LDR sample did not approach the modeled 3.3 V rail');
  check(positive.led_final === 0, 'positive run did not clear the IO48 user LED after observing it on');
  check(positive.trace.dropped === 0 && positive.trace.edges.length === 4, 'positive digital trace did not capture LED on/off edges');

  const negative = run(0);
  equal(negative, expected.negative, 'dark negative control differs from the native Flex model reference');
  check(negative.adc_code === 0 && negative.led_final === 0, 'dark negative control must keep the LED off');
  check(negative.trace.dropped === 0 && negative.trace.edges.length === 0, 'dark negative control emitted a digital edge');

  return {
    status: 'passed',
    runtime: 'compiled-wasm',
    firmware_bytes: firmwareBytes.length,
    cpu_emulation: false,
    source_board: 'HeyPCB ESP32-S3 LDR / GL5528 / ESP32-S3-WROOM-1-N16R8',
    observable: 'source.U3.GPIO1 LDR voltage converted to 12-bit ADC count; source.U3.GPIO48 drives D1 logic state',
    positive_adc_code: positive.adc_code,
    positive_led_edges: positive.trace.edges.length,
    negative_adc_code: negative.adc_code,
    negative_led_edges: negative.trace.edges.length,
    checks: ['portable-firmware-execution', 'native-model-equality', 'pinned-source-mapping', 'bright-output', 'dark-negative-control'],
  };
}
