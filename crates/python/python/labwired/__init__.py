"""Synchronous firmware simulation in virtual time, backed by LabWired Session."""
from dataclasses import dataclass
from decimal import Decimal, ROUND_CEILING
from pathlib import Path
import json
import re

from ._native import Machine, StopReason, ExpectTimeout, NotSupported, NativeSession

__all__ = ['Sim', 'Machine', 'StopReason', 'Match', 'Run', 'ExpectTimeout', 'NotSupported', 'run_firmware']

@dataclass(frozen=True)
class Match:
    text: str
    captures: list
    at: float


@dataclass(frozen=True)
class Run:
    uart: str
    stop_reason: StopReason
    time: float
    cycles: int
    matches: list


def _nanoseconds(value):
    if isinstance(value, bool):
        raise TypeError('duration must be seconds or a duration string')
    if isinstance(value, str):
        match = re.fullmatch(r'\s*(\d+(?:\.\d*)?|\.\d+)\s*(s|ms|us|ns)\s*', value)
        if not match:
            raise ValueError('duration must use s, ms, us, or ns')
        number, unit = match.groups()
        seconds = Decimal(number) * {'s': Decimal(1), 'ms': Decimal('.001'), 'us': Decimal('.000001'), 'ns': Decimal('.000000001')}[unit]
    elif isinstance(value, (int, float, Decimal)):
        seconds = Decimal(str(value))
    else:
        raise TypeError('duration must be numeric seconds or a duration string')
    if not seconds.is_finite() or seconds < 0:
        raise ValueError('duration must be finite and nonnegative')
    nanos = int((seconds * 1_000_000_000).to_integral_value(rounding=ROUND_CEILING))
    if nanos > 2**64 - 1:
        raise ValueError('duration exceeds the supported range')
    return nanos


class Sim:
    """One ELF, one machine. Execution only advances during run_for/expect.

    ``uart`` is a peripheral ID, and ``set_pin`` takes a board_io binding ID.
    ``expect`` and ``read_uart`` consume one shared stream; transcripts do not.
    """
    def __init__(self, elf, *, chip=None, system=None, uart=None):
        if (chip is None) == (system is None):
            raise ValueError('provide exactly one of chip or system')
        chip_path = None
        root = Path(__file__).parent / 'configs'
        system_path = Path(system).resolve() if system is not None else None
        if chip is not None:
            name = str(chip)
            if not re.fullmatch(r'[A-Za-z0-9_-]+', name):
                raise ValueError('chip must be a packaged chip name; use system for custom configs')
            chip_path = root / 'chips' / (name + '.yaml')
            if not chip_path.is_file():
                raise ValueError(f'unknown packaged chip {name!r}')
            candidate = root / 'systems' / (name + '.yaml')
            if candidate.is_file():
                system_path = candidate
        self._session = NativeSession(Path(elf), chip_path, system_path, uart, root)
        self._transcript = ''

    @property
    def closed(self):
        return self._session is None

    def _open(self):
        if self.closed:
            raise RuntimeError('Sim is closed')
        return self._session

    def close(self):
        if not self.closed:
            self._transcript = self._session.uart_transcript()
            self._session.close()
            self._session = None

    def __enter__(self):
        self._open()
        return self

    def __exit__(self, *exc):
        self.close()

    @property
    def time(self):
        return self._open().time

    @property
    def cycles(self):
        return self._open().cycles

    def run_for(self, duration):
        return self._open().run_for(_nanoseconds(duration))

    def expect(self, pattern, timeout='1s'):
        # The core budgets a minimum cycle. Requiring a positive wait avoids
        # calling a zero timeout a time-free poll.
        ns = _nanoseconds(timeout)
        if ns == 0:
            raise ValueError('expect timeout must be positive')
        return Match(*self._open().expect(pattern, ns))

    def send(self, text):
        self.send_bytes(text.encode('utf-8'))

    def send_bytes(self, data):
        if not isinstance(data, (bytes, bytearray, memoryview)):
            raise TypeError('send_bytes expects bytes-like data')
        self._open().send(bytes(data))

    def read_uart(self):
        return bytes(self._open().read_uart()).decode('utf-8', errors='replace')

    def uart_transcript(self):
        return self._transcript if self.closed else self._session.uart_transcript()

    def set_input(self, channel, value):
        self._open().set_input(channel, value)

    def set_inputs(self, values):
        self._open().set_inputs(list(values.items()))

    def list_inputs(self):
        return [dict(device=device, **channel)
                for device, channel in json.loads(self._open().list_inputs())]

    def set_pin(self, binding, active):
        self._open().set_pin(binding, active)

    def read_memory(self, address, length):
        return bytes(self._open().read_memory(address, length))

    def read_u32(self, address):
        return self._open().read_u32(address)

    def write_u32(self, address, value):
        self._open().write_u32(address, value)

    def symbol(self, name):
        return self._open().symbol(name)

    def frames(self):
        frames = json.loads(self._open().frames())
        for frame in frames:
            frame['at'] = frame['at']['secs'] + frame['at']['nanos'] / 1e9
        return frames

    def snapshot(self):
        return self._open().snapshot()

    def restore(self, snapshot):
        self._open().restore(snapshot)

    def inject_can(self, bus, id, data, *, extended=False, fd=False, bitrate_switch=False, remote=False):
        self._open().inject_can(bus, id, list(data), extended, fd, bitrate_switch, remote)


def run_firmware(elf, *, duration='1s', chip=None, system=None, uart=None,
                 expect=(), timeout='1s'):
    """Wait for optional patterns, then run a duration and return a Run record."""
    with Sim(elf, chip=chip, system=system, uart=uart) as sim:
        patterns = [expect] if isinstance(expect, str) else expect
        matches = [sim.expect(pattern, timeout=timeout) for pattern in patterns]
        reason = sim.run_for(duration)
        return Run(sim.uart_transcript(), reason, sim.time, sim.cycles, matches)
