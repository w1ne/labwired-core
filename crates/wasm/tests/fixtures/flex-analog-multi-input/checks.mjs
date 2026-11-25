// SPDX-License-Identifier: MIT
// Portable Wasm firmware + existing catalog analog-source checks.
export function runMultiInputAnalogSourceChecks(WasmFlexRuntime, system, firmwareBytes, expected) {
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
  const run = (socPct, usbPresent) => {
    const runtime = WasmFlexRuntime.new_from_yaml(system);
    try {
      const report = canonical(runtime.run_report());
      check(report.execution_path === 'flex_portable_wasm'
        && report.target_cpu_emulated === false && report.time_unit === 'ns'
        && report.scope_notes_complete === true,
      'the run report must disclose portable execution, ns time, and declared scope');
      if (firstReport === undefined) firstReport = report;
      else equal(report, firstReport, 'the run report changed between input controls');
      const soc = report.ports.find(({ name }) => name === 'soc_pct');
      const usb = report.ports.find(({ name }) => name === 'usb_present');
      check(soc?.unit === '%' && usb?.unit === '1' && usb.initial === 0,
        'named model inputs must expose their catalog units and default value');
      const model = report.models.find(({ id }) => id === 'lipo-battery');
      check(model?.adapter === 'behavior' && model.model_type === 'lipo_charger'
        && model.inputs.length === 2
        && model.inputs.some((binding) => binding.name === 'soc_pct' && binding.signal === 'environment.soc_pct')
        && model.inputs.some((binding) => binding.name === 'usb_present' && binding.signal === 'environment.usb_present'),
      'the run report must preserve the registered model and both input bindings');

      const ports = Object.fromEntries(['soc_pct', 'usb_present', 'battery_mv']
        .map((name) => [name, runtime.port_id(name)]));
      const defaultMv = runtime.read(ports.battery_mv);
      const instance = new WebAssembly.Instance(firmware, { labwired: {
        read: (port) => runtime.read(port),
        write: (port, value) => runtime.write(port, value),
        advance_ns: (delta) => runtime.advance_ns(delta),
      } });
      const result = instance.exports.run(ports.soc_pct, ports.usb_present, ports.battery_mv, socPct, usbPresent);
      const batch = canonical(runtime.analog_trace_snapshot(0));
      check(batch.time_unit === 'ns' && batch.dropped === 0
        && batch.channels.length === 1
        && batch.channels[0].name === 'lipo-battery.voltage_mv'
        && batch.channels[0].unit === 'mV',
      'the existing oscilloscope trace must expose the named catalog output in mV');
      check(batch.samples.length === 2
        && batch.samples[0].time_ns === 0 && batch.samples[0].values[0] === 1875
        && batch.samples[1].time_ns === 1000 && batch.samples[1].values[0] === result,
      'the oscilloscope must show the initial default and the sampled two-input response');
      return {
        soc_pct: socPct,
        usb_present: usbPresent,
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

  const positive = run(50, 1);
  equal(positive, expected.positive, 'charging input differs from the native model reference');
  check(positive.result_mv === 1950, '50% state of charge plus charging must produce 1950 mV');
  const negative = run(50, 0);
  equal(negative, expected.negative, 'disconnected charger differs from the native model reference');
  check(negative.result_mv === 1875, 'disconnected charger must not add the charging offset');
  check(positive.result_mv > negative.result_mv, 'the USB stimulus must affect the measured analog output');
  const highCharge = run(100, 0);
  equal(highCharge, expected.high_charge, 'state-of-charge input differs from the native model reference');
  check(highCharge.result_mv === 2100, '100% state of charge must reach the declared voltage clamp');

  return {
    status: 'passed',
    runtime: 'compiled-wasm',
    firmware_bytes: firmwareBytes.length,
    cpu_emulation: false,
    catalog_device_type: 'lipo_charger',
    default_mv: positive.default_mv,
    positive_mv: positive.result_mv,
    negative_mv: negative.result_mv,
    high_charge_mv: highCharge.result_mv,
    observations: { positive, negative, high_charge: highCharge },
    native_model_equality: true,
    run_report: firstReport,
    checks: ['portable-firmware-execution', 'two-input-routing', 'dimensionless-input-unit', 'catalog-default', 'native-model-equality', 'analog-trace-nanoseconds', 'charging-positive-control', 'charging-negative-control', 'state-of-charge-control'],
  };
}
