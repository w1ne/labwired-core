// SPDX-License-Identifier: MIT
// Portable firmware + pinned HeyPCB LDR sensor-network browser checks.
export function runLdrNetworkChecks(WasmFlexRuntime, system, firmwareBytes, expected) {
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
  const portNames = ['lux', 'sensor_v', 'user_led_drive', 'user_led_state'];
  const instantiate = (runtime) => new WebAssembly.Instance(firmware, { labwired: {
    read: (port) => runtime.read(port),
    write: (port, value) => runtime.write(port, value),
    advance_ns: (delta) => runtime.advance_ns(delta),
  } });

  let firstReport;
  const run = (lux) => {
    const runtime = WasmFlexRuntime.new_from_yaml(system);
    try {
      const report = canonical(runtime.run_report());
      check(report.execution_path === 'flex_portable_wasm'
        && report.target_cpu_emulated === false && report.time_unit === 'ns'
        && report.scope_notes_complete === true,
      'the run report must disclose portable execution, ns time, and declared scope');
      check(report.execution_scope?.firmware === 'portable_behavior'
        && report.execution_scope.original_target_firmware_executed === false
        && report.execution_scope.detailed_target_execution_selected === false
        && report.execution_scope.full_board_coverage === 'not_established'
        && report.execution_scope.chip_parity === 'not_established',
      'the run report must separate portable behavior from target firmware, full-board coverage, and chip parity');
      if (firstReport === undefined) firstReport = report;
      else equal(report, firstReport, 'run report changed between illuminance controls');
      const luxPort = report.ports.find(({ name }) => name === 'lux');
      const sensor = report.ports.find(({ name }) => name === 'sensor_v');
      check(luxPort?.unit === 'lx' && luxPort.initial === 10 && sensor?.unit === 'V',
        'illuminance and SNS_AN1 ports must retain their declared units and reference value');
      const model = report.models.find(({ id }) => id === 'source-sns-an1');
      check(model?.adapter === 'analog'
        && model.inputs.some(({ name, signal }) => name === 'lux' && signal === 'environment.lux')
        && model.outputs.some(({ name, signal }) => name === 'voltage' && signal === 'source.U3.GPIO1'),
      'the run report must preserve the analog sensor network and its bindings');
      check(report.assumptions.some((note) => note.includes('GL55 series manual'))
        && report.unsupported_checks.some((note) => note.includes('below 1 lx or above 100 lx'))
        && report.unsupported_checks.some((note) => note.includes('response time')),
      'the run report must carry the photoresistor data source, range limit, and response-time gap');

      const ports = Object.fromEntries(portNames.map((name) => [name, runtime.port_id(name)]));
      const initialV = runtime.read(ports.sensor_v);
      const result = instantiate(runtime).exports.run(
        ports.lux, ports.sensor_v, ports.user_led_drive, ports.user_led_state, lux,
      ) >>> 0;
      const sensorV = runtime.read(ports.sensor_v);
      const analog = canonical(runtime.analog_trace_snapshot(0));
      const digital = canonical(runtime.digital_trace_snapshot(0));
      check(analog.time_unit === 'ns' && analog.dropped === 0
        && analog.channels.length === 1
        && analog.channels[0].name === 'source-sns-an1.voltage'
        && analog.channels[0].unit === 'V'
        && analog.channels[0].port_names.includes('sensor_v'),
      'the existing oscilloscope must expose the named SNS_AN1 port in V and ns');
      check(digital.time_unit === 'ns' && digital.dropped === 0
        && digital.channels.some(({ signal, port_names }) => signal === 'source.D1.logic_state'
          && port_names.includes('user_led_state'))
        && digital.channels.some(({ signal, port_names }) => signal === 'source.U3.GPIO48'
          && port_names.includes('user_led_drive')),
      'the existing logic analyzer must expose both named user LED ports');
      return {
        result,
        lux,
        initial_v: initialV,
        sensor_v: sensorV,
        adc_code: Math.trunc(sensorV * 4095 / 3.3),
        led_final: runtime.read(ports.user_led_state),
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
          edges: digital.edges.map(({ ch, time_ns, value }) => ({ ch, time_ns, value })),
        },
      };
    } finally {
      runtime.free();
    }
  };

  const bright = run(100);
  equal(bright, expected.bright, 'bright sensor-network workload differs from native reference');
  check(bright.result === bright.adc_code && bright.adc_code >= 2048
    && bright.digital.edges.length === 4,
  'bright light must cross the half-scale threshold and toggle the user LED');
  const dim = run(1);
  equal(dim, expected.dim, 'dim sensor-network workload differs from native reference');
  check(dim.result === dim.adc_code && dim.adc_code < 2048 && dim.digital.edges.length === 0,
    'dim light must stay below the threshold and leave the user LED untouched');

  // 63.2% of the 10 lx -> 100 lx step on the scope trace: C7 through R6 ∥ U4.
  const settled = bright.sensor_v;
  const threshold = bright.initial_v + 0.632 * (settled - bright.initial_v);
  const crossing = bright.analog.samples.find(({ values }) => values[0] >= threshold).time_ns;
  check(crossing >= 300_000 && crossing <= 400_000,
    `C7 transient must reach 63.2% between 300 and 400 µs, got ${crossing} ns`);

  // Darkness is outside the data-sheet range: the run must stop, not invent a reading.
  const refusals = [0, 100_000].map((lux) => {
    const runtime = WasmFlexRuntime.new_from_yaml(system);
    try {
      const ports = Object.fromEntries(portNames.map((name) => [name, runtime.port_id(name)]));
      const before = runtime.read(ports.sensor_v);
      let message = null;
      try {
        instantiate(runtime).exports.run(
          ports.lux, ports.sensor_v, ports.user_led_drive, ports.user_led_state, lux,
        );
      } catch (error) {
        message = String(error?.message ?? error);
      }
      check(message !== null && message.includes('photoresistor illuminance must be within 1..100 lx'),
        `${lux} lx must stop the run with the declared range, got ${message}`);
      let stopped = null;
      try { runtime.read(ports.sensor_v); } catch (error) { stopped = String(error?.message ?? error); }
      check(stopped !== null && stopped.includes('Flex runtime stopped')
        && Number(runtime.time_ns()) === 0,
      'a refused illuminance must stop the run without a reading or a time advance');
      const edges = canonical(runtime.digital_trace_snapshot(0)).edges.length;
      check(edges === 0, 'a refused illuminance must not produce LED edges');
      return { lux, error: message, sensor_v_before: before, read_after: stopped, time_ns: 0, led_edges: edges };
    } finally {
      runtime.free();
    }
  });

  return {
    status: 'passed',
    runtime: 'compiled-wasm',
    firmware_bytes: firmwareBytes.length,
    cpu_emulation: false,
    source_board: 'HeyPCB ESP32-S3 LDR revision A',
    observable: 'SNS_AN1 volts from the U4/R6/C7 network and D1 logic state',
    adc_threshold_code: 2048,
    c7_transient_63pct_crossing_ns: crossing,
    observations: { bright, dim },
    out_of_range_refusals: refusals,
    native_model_equality: true,
    run_report: firstReport,
    checks: ['portable-firmware-execution', 'pinned-divider-network', 'c7-transient-on-oscilloscope',
      'named-analog-and-digital-ports', 'oscilloscope-nanoseconds', 'logic-analyzer-nanoseconds',
      'dim-negative-control', 'bright-positive-control', 'out-of-range-refusal'],
  };
}
