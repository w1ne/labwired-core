// SPDX-License-Identifier: MIT
// Portable firmware + held-out registered TMP36 analog-source checks.
export function runTmp36AnalogSourceChecks(WasmFlexRuntime, system, firmwareBytes, expected) {
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  const canonical = (value) => typeof value === 'bigint' ? Number(value) : Array.isArray(value)
    ? value.map(canonical) : value !== null && typeof value === 'object'
      ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])])) : value;
  const equal = (got, wanted, message) => {
    const actual = JSON.stringify(canonical(got));
    const target = JSON.stringify(canonical(wanted));
    check(actual === target, `${message}; got ${actual}; expected ${target}`);
  };
  const firmware = new WebAssembly.Module(firmwareBytes);
  equal(WebAssembly.Module.imports(firmware).map(({ module, name }) => `${module}.${name}`).sort(),
    ['labwired.advance_ns', 'labwired.read', 'labwired.write'], 'unexpected portable firmware imports');

  let firstReport;
  const run = (temperatureC) => {
    const runtime = WasmFlexRuntime.new_from_yaml(system);
    try {
      const report = canonical(runtime.run_report());
      check(report.execution_path === 'flex_portable_wasm'
        && report.target_cpu_emulated === false && report.time_unit === 'ns'
        && report.scope_notes_complete === true,
      'the run report must disclose portable execution, ns time, and declared scope');
      if (firstReport === undefined) firstReport = report;
      else equal(report, firstReport, 'the run report changed between temperature controls');
      const temperature = report.ports.find(({ name }) => name === 'temperature_c');
      const voltage = report.ports.find(({ name }) => name === 'temperature_mv');
      check(temperature?.unit === '°C' && temperature.initial === 25
        && voltage?.unit === 'mV' && voltage.access === 'read',
      'named temperature and voltage ports must retain their units and access');
      const model = report.models.find(({ id }) => id === 'tmp36-sensor');
      check(model?.adapter === 'behavior' && model.model_type === 'tmp36'
        && model.inputs.some(({ name, signal }) => name === 'temperature' && signal === 'environment.temperature_c')
        && model.outputs.some(({ name, signal }) => name === 'voltage_mv' && signal === 'sensor.output_mv'),
      'the run report must preserve the registered formula model and signal bindings');

      const ports = Object.fromEntries(['temperature_c', 'temperature_mv']
        .map((name) => [name, runtime.port_id(name)]));
      const defaultMv = runtime.read(ports.temperature_mv);
      const instance = new WebAssembly.Instance(firmware, { labwired: {
        read: (port) => runtime.read(port),
        write: (port, value) => runtime.write(port, value),
        advance_ns: (delta) => runtime.advance_ns(delta),
      } });
      const result = instance.exports.run(ports.temperature_c, ports.temperature_mv, temperatureC);
      const batch = canonical(runtime.analog_trace_snapshot(0));
      check(batch.time_unit === 'ns' && batch.dropped === 0
        && batch.channels.length === 1
        && batch.channels[0].name === 'tmp36-sensor.voltage_mv'
        && batch.channels[0].unit === 'mV',
      'the existing oscilloscope trace must expose the named catalog output in mV');
      check(batch.samples.length === 2
        && batch.samples[0].time_ns === 0 && batch.samples[0].values[0] === 750
        && batch.samples[1].time_ns === 1000 && batch.samples[1].values[0] === result,
      'the oscilloscope must show the declared default and sampled response in ns');
      return {
        temperature_c: temperatureC,
        default_mv: defaultMv,
        result_mv: result,
        time_ns: Number(runtime.time_ns()),
        trace: {
          time_unit: batch.time_unit,
          channels: batch.channels,
          dropped: batch.dropped,
          samples: batch.samples.map(({ cycle, time_ns, values }) => ({ cycle, time_ns, values })),
        },
      };
    } finally {
      runtime.free();
    }
  };

  const cold = run(-40);
  equal(cold, expected.cold, 'cold output differs from the native model reference');
  check(cold.result_mv === 100, 'the cold endpoint must produce 100 mV');
  const zero = run(0);
  equal(zero, expected.zero, 'zero-degree output differs from the native model reference');
  check(zero.result_mv === 500, 'zero degrees must produce 500 mV');
  const ambient = run(25);
  equal(ambient, expected.default, 'default output differs from the native model reference');
  check(ambient.result_mv === 750, 'the declared 25 °C default must produce 750 mV');
  const hot = run(125);
  equal(hot, expected.hot, 'hot output differs from the native model reference');
  check(hot.result_mv === 1750, 'the hot endpoint must produce 1750 mV');
  check(cold.result_mv < ambient.result_mv && hot.result_mv > ambient.result_mv,
    'cold and hot controls must move the voltage in opposite directions around the default');

  return {
    status: 'passed',
    runtime: 'compiled-wasm',
    firmware_bytes: firmwareBytes.length,
    cpu_emulation: false,
    catalog_device_type: 'tmp36',
    default_mv: ambient.default_mv,
    cold_mv: cold.result_mv,
    zero_mv: zero.result_mv,
    ambient_mv: ambient.result_mv,
    hot_mv: hot.result_mv,
    observations: { cold, zero, default: ambient, hot },
    native_model_equality: true,
    run_report: firstReport,
    checks: ['portable-firmware-execution', 'registered-formula-model', 'temperature-unit', 'catalog-default', 'native-model-equality', 'analog-trace-nanoseconds', 'cold-negative-control', 'hot-positive-control'],
  };
}
