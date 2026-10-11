#!/usr/bin/env python3
"""Replay bounded power-only JSONL scenarios; every assertion is recorded.

Exit nonzero on assertion, adapter, ngspice, or native runner failure. Firmware,
Linux, RF, converter switching and hardware thermal behavior are not executed.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / 'tools' / 'cosim'))
from smart_basket_power import DEFAULTS

DT = 100_000_000


def replay(command, payload):
    result = subprocess.run(command, input=payload, text=True, capture_output=True, timeout=20)
    if result.returncode:
        raise RuntimeError(f'{command}: exit {result.returncode}: {result.stderr}')
    return result.stdout


def sequences(params):
    for name in ('usb_detach', 'radio_burst', 'low_ack', 'low_timeout', 'short',
                 'undervoltage', 'full_charge'):
        requests = []
        for i in range(60):
            inputs = {'host_load_w': 2.0, 'sensor_load_w': 0.1,
                      'usb_present': name in ('usb_detach', 'short', 'full_charge')}
            cfg = dict(params)
            if name in ('low_ack', 'low_timeout'):
                cfg['initial_soc'] = 0.08
                cfg['shutdown_timeout_s'] = 1.0
            if name == 'undervoltage':
                cfg['initial_soc'] = 0.0
            if name == 'full_charge':
                cfg['initial_soc'] = 1.0
            if name == 'usb_detach':
                inputs['usb_present'] = i < 20 or i >= 40
            if name == 'radio_burst':
                inputs['radio_load_w'] = 2.0 if 20 <= i < 30 else 0.0
            if name == 'low_ack':
                inputs['shutdown_ack'] = i == 3
            if name == 'short':
                inputs['fault_short'] = 20 <= i < 25
                inputs['reset'] = i == 40
            if i == 0:
                inputs.update({f'config.{k}': v for k, v in cfg.items()})
            requests.append({'time_ns': (i+1)*DT, 'dt_ns': DT, 'inputs': inputs})
        yield name, requests


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=HERE / 'artifacts')
    parser.add_argument('--parameters', type=Path,
                        help='JSON object with public config keys; overrides assumptions')
    parser.add_argument('--native-cli', type=Path, default=ROOT / 'target/debug/labwired')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    params = dict(DEFAULTS)
    if args.parameters:
        supplied = json.loads(args.parameters.read_text())
        if not isinstance(supplied, dict) or not set(supplied) <= set(DEFAULTS):
            raise ValueError('parameters must be an object containing public config keys')
        params.update(supplied)
    assertions, rows, traces = [], [], {}

    def check(name, passed, detail):
        assertions.append({'name': name, 'passed': bool(passed), 'detail': detail})

    model_cmd = [sys.executable, str(HERE / 'models/power.py')]
    for name, requests in sequences(params):
        payload = ''.join(json.dumps(r, allow_nan=False, sort_keys=True)+'\n' for r in requests)
        (args.output / f'{name}.inputs.jsonl').write_text(payload)
        output = replay(model_cmd, payload)
        check(f'{name}.deterministic_replay', output == replay(model_cmd, payload),
              'same JSONL replayed in two fresh model processes')
        (args.output / f'{name}.outputs.jsonl').write_text(output)
        decoded = [json.loads(line)['outputs'] for line in output.splitlines()]
        if len(decoded) != len(requests):
            raise RuntimeError(f'{name}: adapter lost a step')
        traces[name] = decoded
        for request, out in zip(requests, decoded):
            rows.append({'scenario': name, 'time_ns': request['time_ns'], **out})
        check(f'{name}.finite_and_bounded', all(
            all(math.isfinite(v) for v in out.values() if type(v) in (int, float))
            and 0 <= out['soc'] <= 1
            and 0 <= out['usb_current_a'] <= params['usb_limit_a']+1e-12
            and abs(out['power_balance_error_w']) < 1e-10 for out in decoded),
            'finite outputs, SOC 0..1, USB input limit, bus power residual < 1e-10 W')
    a = traces['usb_detach']
    check('usb_detach.charging_on_usb', a[10]['charge_current_a'] > 0, a[10])
    check('usb_detach.battery_supplies_detached', a[25]['battery_current_a'] > 0
          and a[25]['charge_current_a'] == 0 and a[25]['usb_current_a'] == 0, a[25])
    check('usb_detach.rails_survive', all(out['rail_5v'] == 5 for out in a), 'all 60 steps')
    a = traces['radio_burst']
    check('radio_burst.more_current', a[25]['battery_current_a'] > a[10]['battery_current_a'],
          {'idle_a': a[10]['battery_current_a'], 'burst_a': a[25]['battery_current_a']})
    check('radio_burst.cell_sag', a[25]['battery_v'] < a[10]['battery_v'],
          {'idle_v': a[10]['battery_v'], 'burst_v': a[25]['battery_v']})
    a = traces['low_ack']
    check('low_ack.requests_before_cutoff', a[0]['shutdown_request'] and a[0]['rail_5v'] == 5, a[0])
    check('low_ack.disables_rails', a[3]['cutoff_latched'] and a[3]['fault_reason'] == 'shutdown_ack', a[3])
    check('low_ack.no_automatic_restart', a[-1]['rail_5v'] == 0, a[-1])
    a = traces['low_timeout']
    check('low_timeout.cuts_without_ack', a[15]['cutoff_latched']
          and a[15]['fault_reason'] == 'shutdown_timeout', a[15])
    a = traces['short']
    check('short.immediate_disconnect', a[20]['rail_5v'] == 0 and a[20]['fault_reason'] == 'short', a[20])
    check('short.stays_off_after_fault_clears', a[30]['cutoff_latched'], a[30])
    check('short.explicit_reset_recovers', a[40]['rail_5v'] == 5 and not a[40]['cutoff_latched'], a[40])
    a = traces['undervoltage']
    check('undervoltage.independent_cutoff', a[0]['cutoff_latched']
          and a[0]['fault_reason'] == 'undervoltage' and a[0]['rail_5v'] == 0, a[0])
    a = traces['full_charge']
    check('full_charge.charge_terminated', all(out['soc'] == 1 and out['charge_current_a'] == 0 for out in a),
          'full cell receives no charging current')

    # Shared JSONL boundary, real libngspice process, independent divider/RC math.
    spice_requests = [{'time_ns': n*5_000_000, 'dt_ns': 5_000_000,
                       'inputs': {'battery_v': 4.0}} for n in range(1, 6)]
    spice_payload = ''.join(json.dumps(r)+'\n' for r in spice_requests)
    (args.output / 'divider_rc.inputs.jsonl').write_text(spice_payload)
    spice_output = replay([sys.executable, str(HERE / 'models/divider.py')], spice_payload)
    (args.output / 'divider_rc.outputs.jsonl').write_text(spice_output)
    measured = [json.loads(line)['outputs']['adc_v'] for line in spice_output.splitlines()]
    comparison = []
    for req, actual in zip(spice_requests, measured):
        expected = 2.0*(1-math.exp(-req['time_ns']/5_000_000))
        comparison.append({'time_ns': req['time_ns'], 'ngspice_adc_v': actual,
                           'analytic_adc_v': expected, 'error_v': actual-expected})
    spice_ok = len(comparison) == 5 and all(abs(r['error_v']) < 0.01 for r in comparison)
    check('divider_rc.ngspice_matches_closed_form', spice_ok,
          '100k/100k divider, 100nF, 4V step: max absolute error < 10mV')

    # Each CLI invocation is a distinct model process and signal store.
    native = []
    for usb in (True, False):
        cmd = [str(args.native_cli), 'cosim-step', str(HERE / 'system.yaml'),
               '--steps', '10', '--json', '--set', f'basket.usb_present={str(usb).lower()}',
               '--set', 'basket.host_load_w=2.0', '--set', 'basket.config.initial_soc=0.7',
               '--set', 'basket.config.usb_limit_a=0.6']
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=20)
        if result.returncode:
            raise RuntimeError(f'native runner failed: {result.stderr}')
        filename = f'native_{"usb" if usb else "battery"}.json'
        (args.output / filename).write_text(result.stdout)
        steps = json.loads(result.stdout)
        final = steps[-1]['outputs']
        ok = (len(steps) == 10 and final['basket.power.rail_5v'] == 5
              and (final['basket.power.soc'] > 0.7 if usb else final['basket.power.soc'] < 0.7)
              and final['basket.power.usb_current_a'] <= 0.6)
        check(f'native.{"usb" if usb else "battery"}.routes_and_steps', ok, final)
        native.append({'command': cmd, 'output': filename, 'step_count': len(steps)})
    with (args.output / 'waveforms.csv').open('w', newline='') as handle:
        writer = csv.DictWriter(handle, fieldnames=list(rows[0]), lineterminator='\n')
        writer.writeheader()
        writer.writerows(rows)
    passed = all(assertion['passed'] for assertion in assertions)
    report = {
        'passed': passed, 'evidence_boundary': 'power-only; no firmware/Linux/RF/thermal/physical hardware',
        'parameter_source': str(args.parameters) if args.parameters else 'documented default assumptions',
        'parameters': params, 'scenario_overrides': {
            'low_ack/low_timeout': {'initial_soc': 0.08, 'shutdown_timeout_s': 1.0},
            'undervoltage': {'initial_soc': 0.0}, 'full_charge': {'initial_soc': 1.0}},
        'scenario_count': len(traces), 'waveform_rows': len(rows),
        'assertions': assertions,
        'divider_rc': {'passed': spice_ok, 'samples': comparison},
        'native_runner': native,
        'model_sha256': hashlib.sha256((ROOT / 'tools/cosim/smart_basket_power.py').read_bytes()).hexdigest(),
        'native_cli_sha256': hashlib.sha256(args.native_cli.read_bytes()).hexdigest(),
    }
    (args.output / 'report.json').write_text(json.dumps(report, indent=2, allow_nan=False)+'\n')
    print(json.dumps({'passed': passed, 'assertions': len(assertions),
                      'waveform_rows': len(rows), 'report': str(args.output / 'report.json')}))
    return 0 if passed else 1


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, RuntimeError, OSError, subprocess.SubprocessError) as exc:
        print(f'scenario error: {exc}', file=sys.stderr)
        sys.exit(1)
