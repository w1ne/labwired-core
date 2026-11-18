// SPDX-License-Identifier: MIT
// Portable firmware + registered NTC analog-source browser checks.
export function runNtcAnalogSourceChecks(WasmFlexRuntime, system, firmwareBytes, expected) {
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
      else equal(report, firstReport, 'run report changed between temperature controls');
      const temperature = report.ports.find(({ name }) => name === 'temperature_c');
      const voltage = report.ports.find(({ name }) => name === 'temperature_mv');
      const alarm = report.ports.find(({ name }) => name === 'hot_alarm_state');
      check(temperature?.unit === '°C' && temperature.initial === 25
        && voltage?.unit === 'mV' && alarm?.kind === 'bool',
      'temperature, analog output, and digital alarm ports must retain their declared types and units');
      const model = report.models.find(({ id }) => id === 'ntc-sensor');
      check(model?.adapter === 'behavior' && model.model_type === 'ntc-thermistor'
        && model.inputs.some(({ name, signal }) => name === 'temperature' && signal === 'environment.temperature_c')
        && model.outputs.some(({ name, signal }) => name === 'voltage_mv' && signal === 'sensor.output_mv'),
      'the run report must preserve the registered NTC model and its bindings');

      const ports = Object.fromEntries(['temperature_c', 'temperature_mv', 'hot_alarm_drive', 'hot_alarm_state']
        .map((name) => [name, runtime.port_id(name)]));
      const instance = new WebAssembly.Instance(firmware, { labwired: {
        read: (port) => runtime.read(port),
        write: (port, value) => runtime.write(port, value),
        advance_ns: (delta) => runtime.advance_ns(delta),
      } });
      const voltageMv = instance.exports.run(
        ports.temperature_c, ports.temperature_mv, ports.hot_alarm_drive, temperatureC,
      );
      const analog = canonical(runtime.analog_trace_snapshot(0));
      const digital = canonical(runtime.digital_trace_snapshot(0));
      check(analog.time_unit === 'ns' && analog.dropped === 0
        && analog.channels.length === 1
        && analog.channels[0].name === 'ntc-sensor.voltage_mv'
        && analog.channels[0].unit === 'mV'
        && analog.channels[0].port_names.includes('temperature_mv'),
      'the existing oscilloscope must expose the named NTC voltage port in mV and ns');
      check(analog.samples.length === 3
        && analog.samples[0].time_ns === 0
        && analog.samples[1].time_ns === 1000
        && analog.samples[2].time_ns === 2000
        && analog.samples[2].values[0] === voltageMv,
      'the NTC output samples must align with firmware host advances');
      check(digital.time_unit === 'ns' && digital.dropped === 0
        && digital.channels.some(({ signal, port_names }) => signal === 'alarm.state'
          && port_names.includes('hot_alarm_state')),
      'the existing logic analyzer must expose the named alarm port');
      return {
        temperature_c: temperatureC,
        voltage_mv: voltageMv,
        alarm_state: runtime.read(ports.hot_alarm_state) === 1,
        time_ns: Number(runtime.time_ns()),
        analog: {
          time_unit: analog.time_unit,
          channels: analog.channels,
          dropped: analog.dropped,
          samples: analog.samples.map(({ cycle, time_ns, values }) => ({ cycle, time_ns, values })),
        },
        digital: {
          time_unit: digital.time_unit,
          channels: digital.channels,
          dropped: digital.dropped,
          edges: digital.edges,
        },
      };
    } finally {
      runtime.free();
    }
  };

  const cold = run(0);
  equal(cold, expected.cold, 'cold NTC workload differs from native reference');
  check(cold.voltage_mv < 2_000 && cold.alarm_state === false,
    'cold thermistor must keep the over-temperature alarm off');
  const ambient = run(25);
  equal(ambient, expected.ambient, 'ambient NTC workload differs from native reference');
  check(ambient.voltage_mv === 1_650 && ambient.alarm_state === false,
    'the 25 °C divider midpoint must remain below the alarm threshold');
  const hot = run(50);
  equal(hot, expected.hot, 'hot NTC workload differs from native reference');
  check(hot.voltage_mv > 2_000 && hot.alarm_state === true,
    'hot thermistor must assert the over-temperature alarm');

  return {
    status: 'passed',
    runtime: 'compiled-wasm',
    firmware_bytes: firmwareBytes.length,
    cpu_emulation: false,
    catalog_device_type: 'ntc-thermistor',
    alarm_threshold_mv: 2_000,
    observations: { cold, ambient, hot },
    native_model_equality: true,
    run_report: firstReport,
    checks: ['portable-firmware-execution', 'registered-ntc-formula', 'named-analog-and-digital-ports',
      'oscilloscope-nanoseconds', 'logic-analyzer-nanoseconds', 'cold-negative-control', 'hot-positive-control'],
  };
}
