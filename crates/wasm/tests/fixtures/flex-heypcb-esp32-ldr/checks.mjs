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

  let runReport;
  const run = (lux) => {
    const runtime = WasmFlexRuntime.new_from_yaml(system);
    try {
      for (const invalidCursor of [-1, 0.5, Number.MAX_SAFE_INTEGER + 1, NaN]) {
        for (const snapshot of [
          () => runtime.digital_trace_snapshot(invalidCursor),
          () => runtime.analog_trace_snapshot(invalidCursor),
        ]) {
          let rejected = false;
          try { snapshot(); } catch { rejected = true; }
          check(rejected, `trace cursor ${String(invalidCursor)} must be rejected`);
        }
      }
      const report = canonical(runtime.run_report());
      check(report.execution_path === 'flex_portable_wasm'
        && report.target_cpu_emulated === false
        && report.time_unit === 'ns'
        && report.scope_notes_complete === true,
      'the project run report must identify portable firmware, nanoseconds, and declared scope');
      check(report.model_name === 'heypcb-esp32-s3-ldr-gl5528'
        && report.assumptions.length === 4
        && report.assumptions.some((assumption) => assumption.includes('Part Knowledge')
          && assumption.includes('GL5528'))
        && report.unsupported_checks.length === 3,
      'the project run report omitted its model assumptions or unsupported checks');
      const sensorModel = report.models.find((model) => model.id === 'source-u4-gl5528');
      check(sensorModel?.adapter === 'behavior' && sensorModel.step_ns === 1000
        && sensorModel.model_type === 'ldr'
        && sensorModel.config?.device_type === 'ldr'
        && sensorModel.inputs.some((binding) => binding.name === 'lux'
          && binding.signal === 'environment.lux')
        && sensorModel.outputs.some((binding) => binding.name === 'voltage_mv'
          && binding.signal === 'source.U3.GPIO1'),
      `the project run report omitted the registered LDR configuration, binding, model type, or time resolution: ${JSON.stringify(sensorModel)}`);
      const ledModel = report.models.find((model) => model.id === 'source-d1-user-led');
      check(ledModel?.config?.behavior?.primitive === 'gpio_device'
        && ledModel.config.behavior.rules.length === 2,
      'the project run report omitted the complete data-driven LED behavior configuration');
      if (runReport === undefined) runReport = report;
      else equal(report, runReport, 'run report changed between positive and negative controls');
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
      const batch = canonical(runtime.digital_trace_snapshot(0));
      const analogBatch = canonical(runtime.analog_trace_snapshot(0));
      check(Number.isSafeInteger(runtime.time_ns()),
        'the project virtual clock must use the existing instrument safe-Number contract');
      check(Number.isSafeInteger(batch.cursor)
        && batch.edges.every((edge) => Number.isSafeInteger(edge.time_ns)),
      'logic-analyzer cursors and nanosecond timestamps must fit the existing instrument Number contract');
      check(Number.isSafeInteger(analogBatch.next_cursor)
        && analogBatch.samples.every((sample) => Number.isSafeInteger(sample.time_ns)),
      'oscilloscope cursors and nanosecond timestamps must fit the existing instrument Number contract');
      const digitalTail = canonical(runtime.digital_trace_snapshot(batch.cursor));
      const analogTail = canonical(runtime.analog_trace_snapshot(analogBatch.next_cursor));
      check(digitalTail.edges.length === 0 && digitalTail.cursor === batch.cursor
        && digitalTail.dropped === 0,
      'logic-analyzer cursor polling must not replay or drop already-read edges');
      check(analogTail.samples.length === 0 && analogTail.next_cursor === analogBatch.next_cursor
        && analogTail.dropped === 0,
      'oscilloscope cursor polling must not replay or drop already-read samples');
      check(analogBatch.time_unit === 'ns'
        && analogBatch.channels.length === 1
        && analogBatch.channels[0].name === 'source-u4-gl5528.voltage_mv'
        && analogBatch.channels[0].unit === 'mV',
      'the registered LDR output or its nanosecond time unit is missing from the oscilloscope trace');
      check(analogBatch.samples.length > 0, 'the LDR oscilloscope trace has no samples');
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
        analog_trace: {
          time_unit: analogBatch.time_unit,
          channels: analogBatch.channels,
          dropped: analogBatch.dropped,
          next_cursor: analogBatch.next_cursor,
          samples: analogBatch.samples.map(({ cycle, time_ns, values }) => ({ cycle, time_ns, values })),
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
  const scopeSummary = (run) => ({
    time_unit: run.analog_trace.time_unit,
    channels: run.analog_trace.channels,
    dropped: run.analog_trace.dropped,
    samples: run.analog_trace.samples.map(({ time_ns, values }) => [time_ns, values[0]]),
  });

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
    analog_trace: {
      positive: scopeSummary(positive),
      negative: scopeSummary(negative),
    },
    native_model_equality: true,
    run_report: runReport,
    checks: ['portable-firmware-execution', 'native-model-equality', 'pinned-source-mapping', 'run-scope-report', 'bright-output', 'dark-negative-control', 'oscilloscope-samples', 'oscilloscope-nanosecond-time-unit'],
  };
}
