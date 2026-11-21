"""Synchronous firmware simulation in virtual time, backed by LabWired Session."""
from dataclasses import dataclass
from decimal import Decimal
from pathlib import Path
import json
import re

from ._native import Machine, StopReason, ExpectTimeout, NotSupported, NativeSession
from ._lower import LowerUnavailable, DiagramError, lower as lower_diagram, meter_readings

__all__ = ['Sim', 'World', 'Machine', 'StopReason', 'Match', 'Run', 'Edge', 'ExpectTimeout', 'NotSupported',
           'LowerUnavailable', 'DiagramError', 'lower_diagram', 'run_firmware']

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


@dataclass(frozen=True)
class Edge:
    """One level change on a watched pad."""
    pad: str
    level: bool
    cycle: int
    time: float


def _pad_ref(pad, pads):
    """A pad as (peripheral, pin): a diagram pin name, 'gpiob:4', or a tuple."""
    if isinstance(pad, (tuple, list)) and len(pad) == 2:
        return str(pad[0]), int(pad[1])
    text = str(pad)
    if text in pads:
        ref = pads[text]
        return ref['peripheral'], int(ref['pin'])
    match = re.fullmatch(r'([A-Za-z0-9_]+):(\d+)', text)
    if match:
        return match.group(1), int(match.group(2))
    known = ', '.join(sorted(pads)) or 'none: the drawing wires no pin of this chip'
    raise ValueError(f'unknown pad {text!r}; use a pin the diagram wires ({known}) or "peripheral:pin"')


class _Edges:
    """Pad watching shared by a Sim and a World machine."""
    def _arm(self, pads, arm):
        self._watched = [_pad_ref(p, self._pads) for p in pads]
        self._watch_names = [p if isinstance(p, str) else f'{p[0]}:{p[1]}' for p in pads]
        self._edge_cursor = 0
        return arm(self._watched)

    def _drain(self, read, hz):
        batch = json.loads(read(self._edge_cursor))
        self._edge_cursor = batch['cursor']
        return [Edge(self._watch_names[e['ch']], bool(e['value']), e['cycle'], e['cycle'] / hz) for e in batch['edges']]


def _nanoseconds(value):
    if isinstance(value, bool):
        raise TypeError('duration must be seconds or a duration string')
    if isinstance(value, str):
        match = re.fullmatch(r'\s*(\d+(?:\.\d*)?|\.\d+)\s*(s|ms|us|ns)\s*', value)
        if not match:
            raise ValueError('duration must use s, ms, us, or ns')
        number, unit = match.groups()
        amount = Decimal(number)
        shift = {'s': 9, 'ms': 6, 'us': 3, 'ns': 0}[unit]
    elif isinstance(value, (int, float, Decimal)):
        amount = Decimal(str(value))
        shift = 9
    else:
        raise TypeError('duration must be numeric seconds or a duration string')
    if not amount.is_finite() or amount < 0:
        raise ValueError('duration must be finite and nonnegative')
    # Decimal arithmetic uses the caller's context, which can round away a
    # positive fraction or trap before we apply the nanosecond ceiling. Shift
    # the exact coefficient instead; only the at-most-20 integer digits need
    # conversion, even for very small or very large Decimal exponents.
    _, digits, exponent = amount.as_tuple()
    if not any(digits):
        return 0
    places = len(digits) + exponent + shift
    if places > 20:
        raise ValueError('duration exceeds the supported range')
    if places <= 0:
        return 1
    nanos = 0
    for digit in digits[:places]:
        nanos = nanos * 10 + digit
    nanos *= 10 ** max(0, places - len(digits))
    if any(digits[places:]):
        nanos += 1
    if nanos > 2**64 - 1:
        raise ValueError('duration exceeds the supported range')
    return nanos


class Sim(_Edges):
    """One ELF, one machine. Execution only advances during run_for/expect.

    ``uart`` is a peripheral ID, and ``set_pin`` takes a board_io binding ID.
    ``expect``, ``read_uart`` and ``read_uart_bytes`` consume one shared stream; transcripts do not.

    ``Sim(elf, diagram="board.json")`` runs a drawing: the diagram is lowered by
    ``labwired-lower`` (the compiler the playground and the hosted API use), its
    analog island and bench instruments included. A diagram with several MCUs
    returns a :class:`World`, and ``elf`` is then a mapping from part id to ELF.
    """
    def __new__(cls, elf=None, *, diagram=None, chip_yaml=None, **kw):
        if diagram is None:
            return super().__new__(cls)
        from ._diagram import open_diagram
        return open_diagram(cls, elf, diagram, chip_yaml, kw)

    def __init__(self, elf, *, chip=None, system=None, diagram=None, chip_yaml=None, uart=None, coverage=False):
        if getattr(self, '_ready', False):
            return  # opened from a diagram by __new__
        if diagram is not None:
            raise TypeError('diagram is handled in __new__')
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
        self._session = NativeSession(Path(elf), chip_path, system_path, uart, root, bool(coverage))
        self._transcript = ''
        self._pads = {}
        self._ready = True

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
        """Drain unread UART output as text. Invalid UTF-8 becomes U+FFFD."""
        return self.read_uart_bytes().decode('utf-8', errors='replace')

    def read_uart_bytes(self):
        """Drain unread UART output as raw bytes, with no decode."""
        return bytes(self._open().read_uart())

    def uart_transcript(self):
        return self._transcript if self.closed else self._session.uart_transcript()

    def set_input(self, channel, value):
        self._open().set_input(channel, value)

    def set_inputs(self, values):
        self._open().set_inputs(list(values.items()))

    def list_inputs(self):
        return [dict(device=device, **channel)
                for device, channel in json.loads(self._open().list_inputs())]

    def set_signal(self, path, value):
        """Set a signal of the analog island, such as ``"ui.touch.pressed"``.

        The value is a number (``True`` and ``False`` count as 1 and 0) and
        stays until set again. A path no circuit reads raises ``ValueError``.
        """
        self._open().set_signal(path, float(value))

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

    def inject_fault(self, kind=None, *, at_cycle=None, register=None, address=None, bit=None,
                     faults=None, run_for=None, until_cycle=None):
        """Run a lockstep fault experiment from the current point.

        Two copies of this Sim are built at the current point: a golden one and
        a faulted one. The faults fire on the faulted copy at their cycles, both
        step one instruction at a time, and the report says what the fault did.
        This Sim itself is not changed.

        One fault: ``kind`` is ``'register_bit_flip'`` (``register`` name or
        index, ``bit``), ``'memory_bit_flip'`` (``address``, ``bit``) or
        ``'instruction_skip'``; ``at_cycle`` defaults to now. Several: pass
        ``faults=[{'at_cycle': ..., 'kind': ..., ...}, ...]``.

        The run lasts ``run_for`` (a duration) or until ``until_cycle``.

        Returns the report as a dict. ``report['verdict']`` is one of
        ``masked``, ``latent``, ``diverged``, ``output_changed``, ``crashed``
        and ``not_injected``.
        """
        session = self._open()
        if (faults is None) == (kind is None):
            raise ValueError('pass kind=... for one fault, or faults=[...]')
        if faults is None:
            fault = {'kind': kind, 'at_cycle': session.cycles if at_cycle is None else at_cycle}
            if register is not None:
                fault['register'] = register
            if address is not None:
                fault['address'] = address
            if bit is not None:
                fault['bit'] = bit
            faults = [fault]
        if (run_for is None) == (until_cycle is None):
            raise ValueError('pass exactly one of run_for or until_cycle')
        plan = {'faults': list(faults), 'until_cycle': until_cycle or 0}
        ns = None if run_for is None else _nanoseconds(run_for)
        return json.loads(session.fault_experiment(json.dumps(plan), ns))

    def fault_verdict(self):
        """Why and where the firmware faulted, as a dict, or ``None`` when it
        did not. ``summary`` is one plain sentence, for example
        ``HardFault escalated from a precise BusFault: data access at
        0x3000_0004 (BFAR valid), at PC 0x0800_006C in `sensor_read`
        (fault_fixture.c:55), called from `main`.`` The other keys are the
        decoded HFSR/CFSR/BFAR/MMFAR, the stacked frame and the symbolized PC.
        Cortex-M only."""
        raw = self._open().fault_verdict()
        return None if raw is None else json.loads(raw)

    def watch(self, *pads):
        """Watch GPIO pads for level changes; returns each pad's level now.

        A pad is a pin the diagram wires (``"PB4"``), ``"gpiob:4"``, or a
        ``(peripheral, pin)`` pair. ``edges()`` then returns what changed.
        """
        return self._arm(pads, lambda refs: self._open().watch_logic(refs))

    def edges(self):
        """Level changes on the watched pads since the last call, oldest first."""
        session = self._open()
        return self._drain(session.logic_edges, session.cpu_hz)

    @property
    def cpu_hz(self):
        return self._open().cpu_hz

    def analog_trace(self):
        """The analog island's waveform as CSV, as `labwired test --analog-trace` writes it."""
        return self._open().analog_trace()

    def meters(self):
        """Readings of every multimeter in the drawing, from the playground's own function.

        One dict per meter: ``meter``, ``mode``, ``unit``, ``value`` (``None``
        when there is no reading), ``display``, ``overload``, ``settled``.
        """
        return meter_readings(self.analog_trace())

    def meter(self, name):
        for reading in self.meters():
            if reading['meter'] == name:
                return reading
        raise KeyError(f'no multimeter {name!r} in the analog trace')

    def coverage(self):
        """Firmware coverage so far, as a dict: per-file lines, per-function
        summary, branch counts, percentages, and the LCOV text under ``lcov``.
        The Sim must be opened with ``coverage=True``."""
        return json.loads(self._open().coverage())


def run_firmware(elf, *, duration='1s', chip=None, system=None, uart=None,
                 expect=(), timeout='1s'):
    """Wait for optional patterns, then run a duration and return a Run record."""
    with Sim(elf, chip=chip, system=system, uart=uart) as sim:
        patterns = [expect] if isinstance(expect, str) else expect
        matches = [sim.expect(pattern, timeout=timeout) for pattern in patterns]
        reason = sim.run_for(duration)
        return Run(sim.uart_transcript(), reason, sim.time, sim.cycles, matches)


from .world import World  # noqa: E402  (World needs the names above)
