"""Behavioral power contract tests; no firmware or Linux execution."""
import importlib.util
import io
import json
import math
from pathlib import Path
import subprocess
import sys

import pytest

HERE = Path(__file__).resolve().parent
MODEL = HERE / 'smart_basket_power.py'


def model():
    spec = importlib.util.spec_from_file_location('smart_basket_power', MODEL)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.PowerModel()


def step(m, n=1, dt=1_000_000_000, **inputs):
    return m.step({'time_ns': n * dt, 'dt_ns': dt, 'inputs': inputs})


def test_jsonl_process_speaks_native_contract():
    message = {'time_ns': 100_000_000, 'dt_ns': 100_000_000,
               'inputs': {'usb_present': True, 'host_load_w': 2.0}}
    result = subprocess.run([sys.executable, str(MODEL)], input=json.dumps(message)+'\n',
                            text=True, capture_output=True, timeout=5)
    assert result.returncode == 0, result.stderr
    out = json.loads(result.stdout)['outputs']
    assert out['rail_5v'] == 5.0
    assert out['charge_current_a'] > 0


def test_usb_budget_supplies_load_before_charging():
    m = model()
    a = step(m, usb_present=True, host_load_w=1.0)
    b = step(m, 2, usb_present=True, host_load_w=6.0)
    assert b['charge_current_a'] < a['charge_current_a']
    assert a['battery_current_a'] < 0
    assert 0 <= b['usb_current_a'] <= 1.5
    assert abs(b['power_balance_error_w']) < 1e-10


def test_usb_insufficiency_supplements_from_battery_without_charging():
    out = step(model(), usb_present=True, host_load_w=5.0,
               **{'config.usb_limit_a': 0.5})
    assert out['rail_5v'] == 5.0
    assert out['battery_current_a'] > 0
    assert out['charge_current_a'] == 0
    assert math.isclose(out['usb_current_a'], 0.5)
    assert abs(out['power_balance_error_w']) < 1e-10


def test_radio_burst_draws_more_current_and_sags_cell():
    a = step(model(), host_load_w=2.0)
    b = step(model(), host_load_w=2.0, radio_load_w=2.0)
    assert b['battery_current_a'] > a['battery_current_a'] > 0
    assert b['battery_v'] < a['battery_v']
    assert b['soc'] < a['soc']
    assert a['charge_current_a'] == 0


def test_empty_cell_has_independent_cutoff_before_software_ack():
    out = step(model(), host_load_w=3.0, **{'config.initial_soc': 0.0})
    assert out['cutoff_latched'] is True
    assert out['rail_5v'] == 0
    assert out['shutdown_request'] is True
    assert out['fault_reason'] == 'undervoltage'


def test_low_battery_request_then_ack_latches_power_off():
    m = model()
    a = step(m, host_load_w=6.2, **{'config.initial_soc': 0.3})
    assert a['shutdown_request'] and not a['cutoff_latched']
    b = step(m, 2, host_load_w=6.2, shutdown_ack=True)
    assert b['cutoff_latched'] and b['fault_reason'] == 'shutdown_ack'
    c = step(m, 3, usb_present=True, enable=True)
    assert c['rail_5v'] == 0 and c['cutoff_latched']
    d = step(m, 4, usb_present=True, reset=True)
    assert d['rail_5v'] == 5 and not d['cutoff_latched']


def test_shutdown_timeout_is_bounded_and_latched():
    m = model()
    a = step(m, host_load_w=2.0, **{'config.initial_soc': 0.08,
                                   'config.shutdown_timeout_s': 2.0})
    assert a['rail_5v'] == 5
    out = step(m, 2, host_load_w=2.0)
    assert out['rail_5v'] == 0
    assert out['fault_reason'] == 'shutdown_timeout'


def test_short_disconnects_rails_and_requires_explicit_recovery():
    m = model()
    out = step(m, usb_present=True, fault_short=True)
    assert out['cutoff_latched'] and out['rail_3v3'] == 0
    assert out['battery_current_a'] == 0  # charger also inhibited on short
    assert step(m, 2, usb_present=True, reset=True, fault_short=True)['cutoff_latched']
    assert step(m, 3, usb_present=True)['cutoff_latched']
    assert not step(m, 4, usb_present=True, reset=True)['cutoff_latched']


def test_reset_cannot_restart_low_cell_without_adequate_usb():
    m = model()
    step(m, **{'config.initial_soc': 0})
    assert step(m, 2, reset=True)['cutoff_latched']
    assert step(m, 3, reset=True, usb_present=True, host_load_w=10,
                **{'config.usb_limit_a': 1.5})['cutoff_latched']


def test_full_cell_terminates_charging_and_soc_is_bounded():
    m = model()
    out = step(m, dt=60_000_000_000, usb_present=True,
               **{'config.initial_soc': 0.999, 'config.capacity_mah': 100})
    for n in range(2, 61):
        out = step(m, n, dt=60_000_000_000, usb_present=True)
    assert 0.999 <= out['soc'] <= 1
    assert out['charge_current_a'] < 0.001
    assert out['battery_v'] <= 4.2 + 1e-12


def test_requested_sensor_power_includes_buck_loss():
    out = step(model(), host_load_w=1, sensor_load_w=0.9,
               **{'config.buck_efficiency': 0.9})
    assert math.isclose(out['rail_input_load_w'], 2.0)
    assert math.isclose(out['requested_load_w'], 1.9)


def test_coulomb_balance_accounts_for_reported_average_current():
    out = step(model(), host_load_w=2, **{'config.initial_soc': 0.6,
                                        'config.capacity_mah': 2000})
    assert math.isclose(out['soc'], 0.6-out['battery_current_a']/7200, abs_tol=1e-12)


def test_disable_does_not_discharge_and_usb_can_charge_latched_cell():
    out = step(model(), enable=False, host_load_w=2)
    assert out['delivered_load_w'] == 0 and out['battery_current_a'] == 0
    m = model()
    step(m, fault_short=True)
    out = step(m, 2, usb_present=True)
    assert out['rail_5v'] == 0 and out['charge_current_a'] > 0


@pytest.mark.parametrize('inputs', [
    {'host_load_w': float('nan')}, {'host_load_w': float('inf')},
    {'host_load_w': -1}, {'host_load_w': True}, {'usb_present': 1},
    {'enable': 'true'}, {'config.capacity_mah': 0}, {'config.initial_soc': 1.1},
    {'config.boost_efficiency': 0}, {'config.cutoff_v': 3.5,
                                     'config.low_battery_v': 3.4},
    {'typo_load_w': 2}, {'config.usb_limit_a': 100},
])
def test_invalid_inputs_fail_without_advancing_state(inputs):
    m = model()
    with pytest.raises(ValueError):
        step(m, **inputs)
    assert step(m)['soc'] == 0.6


@pytest.mark.parametrize('message', [
    {'time_ns': 0, 'dt_ns': 1, 'inputs': {}},
    {'time_ns': 1, 'dt_ns': 0, 'inputs': {}},
    {'time_ns': 1.5, 'dt_ns': 1, 'inputs': {}},
    {'time_ns': True, 'dt_ns': 1, 'inputs': {}},
    {'time_ns': 2, 'dt_ns': 1, 'inputs': {}},
    {'time_ns': 60_000_000_001, 'dt_ns': 60_000_000_001, 'inputs': {}},
    {'time_ns': 1, 'dt_ns': 1, 'inputs': []},
])
def test_invalid_step_times_or_shape_fail(message):
    with pytest.raises(ValueError):
        model().step(message)


def test_duplicate_backward_gap_and_config_changes_are_errors():
    m = model()
    step(m)
    for msg in [{'time_ns': 1_000_000_000, 'dt_ns': 1_000_000_000, 'inputs': {}},
                {'time_ns': 500_000_000, 'dt_ns': 500_000_000, 'inputs': {}},
                {'time_ns': 3_000_000_000, 'dt_ns': 1_000_000_000, 'inputs': {}},
                {'time_ns': 2_000_000_000, 'dt_ns': 1_000_000_000,
                 'inputs': {'config.initial_soc': 0.2}}]:
        with pytest.raises(ValueError):
            m.step(msg)
    assert step(m, 2)['soc'] == 0.6


def test_bad_jsonl_exits_nonzero_without_fake_outputs():
    result = subprocess.run([sys.executable, str(MODEL)], input='{"time_ns":NaN}\n',
                            text=True, capture_output=True, timeout=5)
    assert result.returncode != 0
    assert result.stdout == ''
    assert 'power model error' in result.stderr


def test_native_runner_routes_usb_and_battery_scenarios():
    root = HERE.parents[1]
    cli = root / 'target/debug/labwired'
    if not cli.is_file():
        pytest.skip('build native labwired CLI to enable runner test')
    manifest = root / 'examples/smart-basket-power/system.yaml'
    for usb in (True, False):
        command = [str(cli), 'cosim-step', str(manifest), '--steps', '10', '--json',
                   '--set', f'basket.usb_present={str(usb).lower()}',
                   '--set', 'basket.host_load_w=2.0',
                   '--set', 'basket.config.initial_soc=0.7',
                   '--set', 'basket.config.usb_limit_a=0.6']
        result = subprocess.run(command, capture_output=True, text=True, timeout=15)
        assert result.returncode == 0, result.stderr
        parsed = json.loads(result.stdout)
        outputs = parsed[-1]['outputs']
        assert outputs['basket.power.rail_5v'] == 5.0
        assert 0 < outputs['basket.power.soc'] < 1
        assert outputs['basket.power.soc'] > 0.7 if usb else outputs['basket.power.soc'] < 0.7
        assert outputs['basket.power.usb_current_a'] <= 0.6
        assert outputs['basket.power.charge_current_a'] > 0 if usb else outputs['basket.power.charge_current_a'] == 0


def test_shared_scenario_driver_emits_assertions_csv_and_ngspice(tmp_path):
    root = HERE.parents[1]
    driver = root / 'examples/smart-basket-power/run_scenarios.py'
    result = subprocess.run([sys.executable, str(driver), '--output', str(tmp_path)],
                            capture_output=True, text=True, timeout=30)
    assert result.returncode == 0, result.stderr
    report = json.loads((tmp_path / 'report.json').read_text())
    assert report['passed'] is True
    assert len(report['assertions']) >= 12
    assert report['divider_rc']['passed'] is True
    assert report['scenario_count'] >= 5
    assert (tmp_path / 'waveforms.csv').read_text().startswith('scenario,time_ns,')
    assert b'\r' not in (tmp_path / 'waveforms.csv').read_bytes()
    for jsonl in tmp_path.glob('*.jsonl'):
        assert jsonl.read_text().strip()


def test_driver_returns_nonzero_and_records_failed_assertions(tmp_path):
    root = HERE.parents[1]
    parameters = tmp_path / 'no-usb-budget.json'
    parameters.write_text(json.dumps({'usb_limit_a': 0}))
    output = tmp_path / 'failure-report'
    result = subprocess.run([sys.executable, str(root / 'examples/smart-basket-power/run_scenarios.py'),
                             '--parameters', str(parameters), '--output', str(output)],
                            capture_output=True, text=True, timeout=30)
    assert result.returncode == 1
    report = json.loads((output / 'report.json').read_text())
    assert not report['passed']
    assert any(a['name'] == 'usb_detach.charging_on_usb' and not a['passed']
               for a in report['assertions'])


def test_usb_charge_discharge_energy_balance_is_independent_of_internal_residual():
    # Check the electrical ledger from observables with declared independent
    # efficiencies, including USB source loss, charging loss, and boost loss.
    for inputs in ({'usb_present': True, 'host_load_w': 2},
                   {'usb_present': True, 'host_load_w': 6,
                    'config.usb_limit_a': 0.5},
                   {'usb_present': False, 'host_load_w': 3}):
        out = step(model(), **inputs)
        battery = out['battery_power_w']
        usb_bus = out['usb_current_a'] * 5 * 0.95
        battery_bus = battery*0.9 if battery >= 0 else battery/0.95
        assert math.isclose(usb_bus+battery_bus, out['delivered_rail_input_w'], abs_tol=1e-12)
        assert math.isclose(out['battery_current_a'] * out['battery_terminal_interval_v'],
                            battery, abs_tol=1e-12)


def test_depleted_cell_blocks_usb_rails_and_usb_alone_cannot_reset():
    m = model()
    out = step(m, usb_present=True, host_load_w=2, **{'config.initial_soc': 0})
    assert out['cutoff_latched'] and out['rail_5v'] == 0
    assert out['charge_current_a'] > 0
    out = step(m, 2, usb_present=True, reset=True, host_load_w=2)
    assert out['cutoff_latched'] and out['rail_5v'] == 0


def test_dl33_dl30_fixture_thresholds_route_as_inputs():
    m = model()
    out = step(m, host_load_w=2, **{'config.initial_soc': 0.08,
                                  'config.low_battery_v': 3.3,
                                  'config.cutoff_v': 3.0})
    assert out['shutdown_request'] and not out['cutoff_latched']


def test_empty_source_cannot_report_regulated_rails_below_ocv_cutoff():
    out = step(model(), host_load_w=2, **{'config.initial_soc': 0,
                                       'config.cutoff_v': 2.5})
    assert out['cutoff_latched'] and out['rail_5v'] == 0
    assert out['fault_reason'] == 'source_depleted'


def test_source_exhaustion_turns_off_end_of_interval_rails():
    m = model()
    out = m.step({'time_ns': 60_000_000_000, 'dt_ns': 60_000_000_000,
                  'inputs': {'host_load_w': 2, 'config.initial_soc': 0.005,
                             'config.capacity_mah': 100, 'config.cutoff_v': 2.5,
                             'config.shutdown_timeout_s': 120}})
    assert out['soc'] == 0 and out['rail_5v'] == 0
    assert out['fault_reason'] == 'source_depleted'


def test_failed_rerun_invalidates_previous_success_receipt(tmp_path):
    output = tmp_path / 'artifacts'
    output.mkdir()
    report = output / 'report.json'
    report.write_text('{"passed": true}')
    parameters = tmp_path / 'invalid.json'
    parameters.write_text('{"unknown_key": 1}')
    root = HERE.parents[1]
    result = subprocess.run([sys.executable, str(root / 'examples/smart-basket-power/run_scenarios.py'),
                             '--parameters', str(parameters), '--output', str(output)],
                            capture_output=True, text=True, timeout=30)
    assert result.returncode == 1
    assert not json.loads(report.read_text())['passed']
