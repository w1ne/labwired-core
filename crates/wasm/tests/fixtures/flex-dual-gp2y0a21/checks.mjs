// SPDX-License-Identifier: MIT
// Run portable Wasm firmware against two independent instances of one catalog model.
export function runDualGp2y0a21Checks(WasmFlexRuntime, system, firmwareBytes, expected) {
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
    'the report must identify portable scope and leave target claims unestablished');
    const sensors = report.models.filter(({ model_type }) => model_type === 'gp2y0a21');
    check(sensors.length === 2
      && sensors.map(({ id }) => id).sort().join(',') === 'left-sensor,right-sensor',
    'the report must show two instances of the same registered catalog model');
    check(report.assumptions.some((item) => item.includes('Sharp GP2Y0A21YK0F datasheet, Fig. 2')
      && item.includes('https://global.sharp/products/device/lineup/data/pdf/datasheet/gp2y0a21yk_e.pdf'))
      && report.unsupported_checks.some((item) => item.includes('cross-talk'))
      && report.manifest_sha256 === expected.run_report.manifest_sha256,
    'the report must retain source, unresolved behavior, and the exact manifest digest');

    const ports = Object.fromEntries([
      'left_distance_mm', 'left_distance_mv', 'left_near_drive', 'left_near_led',
      'right_distance_mm', 'right_distance_mv', 'right_near_drive', 'right_near_led',
    ].map((name) => [name, runtime.port_id(name)]));
    const instance = new WebAssembly.Instance(firmware, { labwired: {
      read: (port) => runtime.read(port),
      write: (port, value) => runtime.write(port, value),
      advance_ns: (delta) => runtime.advance_ns(delta),
    } });
    const inputs = [[100, 800], [800, 100], [700, 200]];
    const observations = inputs.map(([left_mm, right_mm]) => {
      const packed = instance.exports.run(
        ports.left_distance_mm, ports.left_distance_mv, ports.left_near_drive,
        ports.right_distance_mm, ports.right_distance_mv, ports.right_near_drive,
        left_mm, right_mm,
      );
      const observation = {
        left_distance_mm: left_mm,
        left_distance_mv: runtime.read(ports.left_distance_mv),
        left_near_led: runtime.read(ports.left_near_led) !== 0,
        right_distance_mm: right_mm,
        right_distance_mv: runtime.read(ports.right_distance_mv),
        right_near_led: runtime.read(ports.right_near_led) !== 0,
      };
      check(packed === observation.left_distance_mv * 10_000 + observation.right_distance_mv,
        'portable firmware did not return both independent sensor readings');
      return observation;
    });
    equal(observations, expected.observations,
      'browser observations differ from the native two-sensor reference');

    const analog = canonical(runtime.analog_trace_snapshot(0));
    check(analog.time_unit === 'ns' && analog.dropped === 0 && analog.channels.length === 2,
      'the existing oscilloscope must retain both named sensor channels');
    check(analog.channels[0].name === 'left-sensor.voltage_mv'
      && analog.channels[0].port_names.includes('left_distance_mv')
      && analog.channels[1].name === 'right-sensor.voltage_mv'
      && analog.channels[1].port_names.includes('right_distance_mv'),
    'each oscilloscope channel must retain its own model and port name');
    const analogEvidence = {
      time_unit: analog.time_unit,
      channels: analog.channels,
      dropped: analog.dropped,
      samples: analog.samples.map(({ cycle, time_ns, values }) => ({ cycle, time_ns, values })),
    };
    equal(analogEvidence, expected.analog_trace,
      'both browser analog traces must match native evidence');

    const digital = canonical(runtime.digital_trace_snapshot(0));
    check(digital.time_unit === 'ns' && digital.dropped === 0
      && digital.channels.some(({ port_names }) => port_names.includes('left_near_led'))
      && digital.channels.some(({ port_names }) => port_names.includes('right_near_led')),
    'the existing logic analyzer must retain both named indicator channels');
    const digitalEvidence = {
      time_unit: digital.time_unit,
      channels: digital.channels,
      dropped: digital.dropped,
      edges: digital.edges,
    };
    equal(digitalEvidence, expected.digital_trace,
      'both browser digital traces and positive/negative control transitions must match native evidence');

    return {
      status: 'passed',
      runtime: 'compiled-wasm',
      firmware_bytes: firmwareBytes.length,
      cpu_emulation: false,
      model_instances: sensors.map(({ id, model_type }) => ({ id, model_type })),
      observations,
      time_ns: runtime.time_ns(),
      traces: { analog: analogEvidence, digital: digitalEvidence },
      run_report: report,
      native_model_equality: true,
      checks: ['portable-firmware-execution', 'same-model-independent-instances',
        'named-ports-and-traces', 'nanosecond-instruments', 'source-and-scope-report',
        'positive-and-negative-controls', 'native-model-equality'],
    };
  } finally {
    runtime.free();
  }
}
