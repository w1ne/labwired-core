from pathlib import Path
import os
import subprocess
import sys
import pytest
try:
    import labwired
except ImportError:
    labwired = None

ROOT = Path(__file__).resolve().parents[3]
ELF = ROOT / 'tests/fixtures/uart-ok-thumbv7m.elf'
CHIP = 'ci-fixture-cortex-m3-uart1'

def new(**kw):
    return labwired.Sim(ELF, chip=CHIP, **kw)

def test_sim_api_available():
    assert hasattr(labwired, 'Sim'), 'Session-backed Sim API missing'

def test_real_firmware_stream_snapshot():
    with new() as s:
        snap = s.snapshot()
        match = s.expect('(O)K', timeout='1ms')
        assert match.text == 'OK' and match.captures == ['O']
        assert s.uart_transcript() == 'OK\n'
        assert s.read_uart() == '\n'
        assert s.read_uart() == ''
        with pytest.raises(labwired.ExpectTimeout):
            s.expect('OK', timeout='1us')
        s.restore(snap)
        assert s.uart_transcript() == ''
        assert s.expect('OK', timeout=.001).text == 'OK'
    assert s.closed
    assert s.uart_transcript() == 'OK\n'
    with pytest.raises(RuntimeError, match='closed'):
        s.run_for('1ms')

def test_memory_time_errors_and_ownership():
    with new() as s, new() as other:
        assert s.run_for('0ns').kind == 'reached'
        assert s.time == 0
        for bad in ['-1ms', 'wat', float('nan'), float('inf'), -1, True]:
            with pytest.raises((ValueError, TypeError)):
                s.run_for(bad)
        s.write_u32(0x20000000, 0x12345678)
        snap = s.snapshot()
        s.write_u32(0x20000000, 1)
        s.restore(snap)
        assert s.read_u32(0x20000000) == 0x12345678
        assert s.read_memory(0x20000000, 4) == b'\x78\x56\x34\x12'
        with pytest.raises(RuntimeError, match='different session'):
            other.restore(snap)
        with pytest.raises(ValueError):
            s.read_u32('missing_symbol')
        with pytest.raises(ValueError):
            s.set_input('missing', 1)
        with pytest.raises(ValueError):
            s.set_pin('missing', True)
        with pytest.raises(ValueError):
            s.inject_can('missing', 0x123, b'a')
        assert s.symbol('missing') is None
        assert s.frames() == []
        s.set_inputs({})
        s.send('hi')
        s.send_bytes(b'hi')

def test_firmware_halt_reason():
    with labwired.Sim(ROOT / 'tests/fixtures/uart-then-bkpt-thumbv7m.elf', chip=CHIP) as s:
        assert s.run_for('1ms').kind == 'halted'
        assert s.cycles == 8 and s.time < .001

def test_named_uart_and_bad_selection():
    with new(uart='uart1') as s:
        assert s.expect('OK', timeout='1ms').text == 'OK'
    with pytest.raises(ValueError):
        new(uart='missing')

def test_helper():
    assert labwired.run_firmware(ELF, chip=CHIP, duration='1ms').uart.startswith('OK')

def test_plugin_subprocess(tmp_path):
    (tmp_path / 'test_sample.py').write_text(f'''
try:
    import labwired
except ImportError:
    labwired = None

def test_fail(sim):
    with sim({str(ELF)!r}) as machine:
        machine.expect("OK", timeout="1ms")
    assert False

def test_skip(sim):
    raise labwired.NotSupported("radio")

created = []
def test_factory_owns_lifetime(sim):
    created.append(sim({str(ELF)!r}))

def test_factory_cleanup_completed():
    assert created[0].closed

''')
    result = subprocess.run([sys.executable, '-m', 'pytest', '-q', str(tmp_path), '--labwired-chip', CHIP, '--junitxml', str(tmp_path/'result.xml')], text=True, capture_output=True)
    assert result.returncode == 1, result.stdout + result.stderr
    assert '1 failed, 2 passed, 1 skipped' in result.stdout
    assert 'LabWired UART' in result.stdout and 'OK' in result.stdout
    assert 'OK' in (tmp_path/'result.xml').read_text()

def test_smart_ring_real_bus_input_and_pin(tmp_path):
    # Leave one button exposing pressed, as in the core Session input tests.
    text = (ROOT / 'examples/nrf54l15-smart-ring/system.yaml').read_text()
    text = text.split('  # Charger-attached detect.')[0]
    text = text.replace('../../configs/chips/nrf54l15.yaml', str(ROOT / 'configs/chips/nrf54l15.yaml'))
    system = tmp_path / 'ring.yaml'
    system.write_text(text)
    with labwired.Sim(ROOT / 'tests/fixtures/nrf54l15-smart-ring.elf',
                      system=system) as s:
        s.set_input('pressed', 1.0)
        s.run_for('1us')
        assert s.read_u32(0x500D8200 + 0x00C) & (1 << 13)
        s.set_inputs({'pressed': 0.0})
        s.run_for('1us')
        assert not s.read_u32(0x500D8200 + 0x00C) & (1 << 13)
        s.set_pin('touch', True)
        assert s.read_u32(0x500D8200 + 0x00C) & (1 << 13)
        s.set_pin('touch', False)
        assert not s.read_u32(0x500D8200 + 0x00C) & (1 << 13)
        s.expect('probe done', timeout='20ms')
        transcript = s.uart_transcript()
        assert 'id=0x24 ack=Y [OK]' in transcript
        assert 'id=0x0117 ack=Y [OK]' in transcript
        frames = s.frames()
        assert any(f['bus'] == 'twi21' for f in frames)
        assert all(isinstance(f['at'], float) for f in frames)
        assert s.frames() == []

def test_run_firmware_preserves_actual_stop_reason():
    run = labwired.run_firmware(ROOT / 'tests/fixtures/uart-then-bkpt-thumbv7m.elf', chip=CHIP, duration='1ms')
    assert run.stop_reason.kind == 'halted'
    assert run.uart == 'OK\n'
    assert run.cycles > 0 and run.time > 0

def test_installed_catalog_outside_checkout(tmp_path):
    import shutil
    firmware = tmp_path / 'firmware.elf'
    shutil.copyfile(ELF, firmware)
    script = f'''
from pathlib import Path
import labwired
assert "site-packages" in labwired.__file__, labwired.__file__
with labwired.Sim("firmware.elf", chip={CHIP!r}) as s:
    assert s.expect("OK", timeout="1ms").text == "OK"
'''
    env = dict(os.environ, LABWIRED_CONFIG_DIR='/does/not/exist', PYTHONPATH='')
    result = subprocess.run([sys.executable, '-c', script], cwd=tmp_path, env=env, text=True, capture_output=True)
    assert result.returncode == 0, result.stdout + result.stderr

@pytest.mark.parametrize('duration', ['0.001s', '1ms', '1000us', '1000000ns', .001])
def test_equivalent_durations(duration):
    with new() as s:
        reason = s.run_for(duration)
        assert reason.kind == 'reached'
        assert s.cycles >= 80_000
        assert .001 <= s.time < .00101


def test_legacy_machine_exports_real_execution():
    machine = labwired.Machine(str(ROOT / 'tests/fixtures/stm32f401-blinky.elf'))
    machine.write_register(0, 123)
    assert machine.read_register(0) == 123
    reason = machine.step(10)
    assert isinstance(reason, labwired.StopReason)
    assert reason.kind == 'max_steps_reached'

def test_helper_expect_sequence():
    run = labwired.run_firmware(ELF, chip=CHIP, expect=['O', 'K'], timeout='1ms', duration='1us')
    assert run.uart.startswith('OK')
    assert [match.text for match in run.matches] == ['O', 'K']


def test_unknown_or_missing_constructor_options():
    with pytest.raises(ValueError):
        labwired.Sim(ELF)
    with pytest.raises(ValueError):
        labwired.Sim(ELF, chip=CHIP, system='missing.yaml')
    with pytest.raises(ValueError):
        labwired.Sim(ELF, chip='../escape')
    with new() as s:
        with pytest.raises(ValueError):
            s.expect('OK', timeout='0s')
        assert s.time == 0

def test_buffered_expect_does_not_advance_time():
    with new() as s:
        s.run_for('1ms')
        now = s.time
        assert s.expect('O', timeout='1ms').text == 'O'
        assert s.expect('K', timeout='1ms').text == 'K'
        assert s.time == now


def test_python_send_routes_and_preserves_byte_order():
    with labwired.Sim(ROOT / 'tests/fixtures/tier1/stm32f103.elf', chip='stm32f103', uart='uart2') as s:
        s.write_u32(0x40021018, 1 << 14)  # Enable USART1 so the untouched RX check is meaningful.
        s.send('A')
        s.send_bytes(b'\x00\xffB')
        assert [s.read_u32(0x40004404) for _ in range(4)] == [65, 0, 255, 66]
        assert s.read_u32(0x40013804) == 0
        assert s.read_u32(0x40004804) == 0

def test_list_inputs_discovers_board_channels():
    with labwired.Sim(ROOT / 'tests/fixtures/nrf54l15-smart-ring.elf',
                      system=ROOT / 'examples/nrf54l15-smart-ring/system.yaml') as s:
        channels = s.list_inputs()
        assert any(c['device'] == 'touch' and c['key'] == 'pressed' for c in channels)
        assert any(c['device'] == 'charger_detect' and c['key'] == 'pressed' for c in channels)

def test_system_can_reference_bare_packaged_chip(tmp_path):
    system = tmp_path / 'board.yaml'
    system.write_text(f'name: bare-chip-board\nchip: {CHIP}\n')
    with labwired.Sim(ELF, system=system) as s:
        assert s.expect('OK', timeout='1ms').text == 'OK'

def test_unknown_uart_text_cannot_turn_validation_error_into_skip():
    try:
        with pytest.raises(ValueError):
            new(uart='not supported')
    except labwired.NotSupported as error:
        pytest.fail(f'invalid UART incorrectly classified as unsupported: {error}')
