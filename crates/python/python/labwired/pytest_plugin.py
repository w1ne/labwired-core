"""Auto-loaded pytest integration for Session-backed firmware tests."""
import pytest
from . import Sim, NotSupported


def pytest_addoption(parser):
    parser.addoption('--labwired-chip', help='Default packaged chip for the sim factory')


@pytest.fixture
def sim(request):
    """Factory owning every Sim it creates; accepts the same options as Sim."""
    sessions = []
    request.node._labwired_sessions = sessions

    def factory(elf, **kwargs):
        if 'chip' not in kwargs and 'system' not in kwargs:
            kwargs['chip'] = request.config.getoption('--labwired-chip')
        machine = Sim(elf, **kwargs)
        sessions.append(machine)
        return machine

    yield factory
    for machine in sessions:
        machine.close()


@pytest.hookimpl(hookwrapper=True)
def pytest_runtest_makereport(item, call):
    outcome = yield
    report = outcome.get_result()
    if call.excinfo is not None and call.excinfo.errisinstance(NotSupported):
        report.outcome = 'skipped'
        report.longrepr = (str(item.path), item.location[1], f'Skipped: {call.excinfo.value}')
    if report.failed:
        for index, machine in enumerate(getattr(item, '_labwired_sessions', []), 1):
            text = machine.uart_transcript()
            report.sections.append((f'LabWired UART {index}', text))
            # JUnit properties survive even when captured output is disabled.
            prop = (f'labwired.uart.{index}', text)
            item.user_properties.append(prop)
            report.user_properties.append(prop)
