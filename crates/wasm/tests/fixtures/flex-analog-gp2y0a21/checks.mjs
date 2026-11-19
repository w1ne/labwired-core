// SPDX-License-Identifier: MIT
// Actual portable Wasm workload + registered analog source + shared instruments.
export function runGp2y0a21Checks(WasmFlexRuntime, system, firmwareBytes, expected) {
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

  const runtime = WasmFlexRuntime.new_from_yaml(system);
  try {
    const report = canonical(runtime.run_report());
    check(report.execution_path === 'flex_portable_wasm'
      && report.target_cpu_emulated === false && report.time_unit === 'ns'
      && report.scope_notes_complete === true
      && report.execution_scope.original_target_firmware_executed === false
      && report.execution_scope.detailed_target_execution_selected === false
      && report.execution_scope.full_board_coverage === 'not_established'
      && report.execution_scope.chip_parity === 'not_established',
    'the run report must disclose portable scope and the unestablished target claims');
    check(report.assumptions.length === 3
      && report.assumptions.some((item) => item.includes('Sharp GP2Y0A21YK0F datasheet, Fig. 2')
        && item.includes('https://global.sharp/products/device/lineup/data/pdf/datasheet/gp2y0a21yk_e.pdf'))
      && report.unsupported_checks.length === 4
      && report.unsupported_checks.some((item) => item.includes('38 ms'))
      && report.unsupported_checks.some((item) => item.includes('physical behavior')),
    'the run report must expose source assumptions and unsupported behavior');
    const sensor = report.models.find(({ id }) => id === 'gp2y0a21-sensor');
    check(sensor?.model_type === 'gp2y0a21'
      && sensor.inputs.some(({ name, signal }) => name === 'distance'
        && signal === 'environment.distance_mm')
      && sensor.outputs.some(({ name, signal }) => name === 'voltage_mv'
        && signal === 'sensor.output_mv'),
    'the run report must retain the registered sensor identity and bindings');
    const distancePort = report.ports.find(({ name }) => name === 'distance_mm');
    const voltagePort = report.ports.find(({ name }) => name === 'distance_mv');
    check(distancePort?.unit === 'mm' && distancePort.initial === 100
      && voltagePort?.unit === 'mV' && voltagePort.access === 'read',
    'named model ports must retain their units and access');
    check(report.manifest_sha256 === expected.run_report.manifest_sha256,
      'run report manifest fingerprint differs from the pinned native input');

    const ports = Object.fromEntries(['distance_mm', 'distance_mv', 'near_drive', 'near_led']
      .map((name) => [name, runtime.port_id(name)]));
    const instance = new WebAssembly.Instance(firmware, { labwired: {
      read: (port) => runtime.read(port),
      write: (port, value) => runtime.write(port, value),
      advance_ns: (delta) => runtime.advance_ns(delta),
    } });
    const points = [
      [100, 2300], [150, 1650], [200, 1300], [300, 930], [400, 730],
      [500, 600], [600, 510], [700, 440], [800, 400],
    ];
    const observations = points.map(([distance_mm, expected_mv]) => {
      const sensor_mv = instance.exports.run(
        ports.distance_mm, ports.distance_mv, ports.near_drive, distance_mm,
      );
      const near_led = runtime.read(ports.near_led) !== 0;
      check(sensor_mv === expected_mv,
        `${distance_mm} mm produced ${sensor_mv} mV; expected rounded datasheet graph point ${expected_mv} mV`);
      check(near_led === (sensor_mv >= 1500),
        `near indicator did not follow the portable firmware threshold at ${distance_mm} mm`);
      return { distance_mm, sensor_mv, near_led };
    });
    equal(observations, expected.observations,
      'browser firmware observations differ from the native Flex reference');

    const analog = canonical(runtime.analog_trace_snapshot(0));
    check(analog.time_unit === 'ns' && analog.dropped === 0
      && analog.channels.length === 1
      && analog.channels[0].name === 'gp2y0a21-sensor.voltage_mv'
      && analog.channels[0].unit === 'mV'
      && analog.channels[0].port_names.includes('distance_mv'),
    'the existing oscilloscope must expose the named mV source channel');
    check(analog.samples.length === 19
      && analog.samples[0].time_ns === 0 && analog.samples[0].values[0] === 2300
      && analog.samples.at(-1).time_ns === 18000 && analog.samples.at(-1).values[0] === 400,
    'the oscilloscope must retain the initial and sampled output in nanoseconds');

    const digital = canonical(runtime.digital_trace_snapshot(0));
    const indicator = digital.channels.find(({ port_names }) => port_names.includes('near_led'));
    check(digital.time_unit === 'ns' && digital.dropped === 0 && indicator,
      'the existing logic analyzer must expose the named indicator channel');
    equal(digital.edges.filter(({ ch }) => ch === indicator.id).map(({ time_ns, value }) => ({ time_ns, value })),
      [{ time_ns: 2000, value: 1 }, { time_ns: 6000, value: 0 }],
      'the logic analyzer must capture the named firmware-driven near/far transitions');

    return {
      status: 'passed',
      runtime: 'compiled-wasm',
      firmware_bytes: firmwareBytes.length,
      cpu_emulation: false,
      device_type: 'gp2y0a21',
      observations,
      traces: {
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
      },
      run_report: report,
      native_model_equality: true,
      checks: ['portable-firmware-execution', 'registered-sharp-source', 'named-ports',
        'run-scope-and-assumptions', 'analog-trace-nanoseconds', 'digital-trace-nanoseconds',
        'near-far-positive-and-negative-controls', 'native-model-equality'],
    };
  } finally {
    runtime.free();
  }
}
