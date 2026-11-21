"""ngspice as a LabWired co-simulation model.

Speaks the `external_process` JSONL contract (see docs/cosimulation_plugins.md):
one `{"time_ns", "dt_ns", "inputs": {...}}` line in, one `{"outputs": {...}}`
line out, in lockstep with the firmware clock. The circuit is any SPICE netlist
ngspice accepts, so open-source device models (`.include`/`.lib`) work as-is.

Mechanics: libngspice is loaded in-process (ctypes). Each step alters the
named voltage sources to the requested values, places a breakpoint at the
step's end time, and runs/resumes the transient analysis to it. Probed node
voltages are read back from ngspice's vectors. No threads, no wall clock:
the same input sequence always yields the same outputs.

Use from a model stub:

    from labwired_ngspice import serve
    serve(open("rc.cir").read(),
          sources={"gpio": "Vgpio"},        # LabWired input name -> SPICE source
          probes={"v_out": "v(out)"},       # LabWired output name -> node/vector
          vdd=3.3)                          # bool inputs map to vdd / 0 V

Requires libngspice (Debian/Ubuntu: `apt install libngspice0`).
"""
from __future__ import annotations

import ctypes
import ctypes.util
import json
import math
import os
import sys
from typing import Callable, Dict, Iterable, Optional, TextIO

__all__ = ["NgSpice", "serve", "NgSpiceError", "BridgeCircuit", "serve_bridge"]


class NgSpiceError(RuntimeError):
    """ngspice reported a fatal condition or refused a command."""


# --- libngspice ABI ---------------------------------------------------------

class _VectorInfo(ctypes.Structure):
    _fields_ = [
        ("v_name", ctypes.c_char_p),
        ("v_type", ctypes.c_int),
        ("v_flags", ctypes.c_short),
        ("v_realdata", ctypes.POINTER(ctypes.c_double)),
        ("v_compdata", ctypes.c_void_p),
        ("v_length", ctypes.c_int),
    ]


_SEND_CHAR = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_void_p)
_SEND_STAT = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_void_p)
_CONTROLLED_EXIT = ctypes.CFUNCTYPE(
    ctypes.c_int, ctypes.c_int, ctypes.c_bool, ctypes.c_bool, ctypes.c_int, ctypes.c_void_p
)


def _load_lib() -> ctypes.CDLL:
    candidates = [
        os.environ.get("LABWIRED_NGSPICE_LIB"),
        ctypes.util.find_library("ngspice"),
        "libngspice.so.0",
        "libngspice.so",
        "libngspice.0.dylib",
        "ngspice.dll",
    ]
    last = None
    for c in candidates:
        if not c:
            continue
        try:
            return ctypes.CDLL(c)
        except OSError as e:  # pragma: no cover - depends on host
            last = e
    raise NgSpiceError(
        "libngspice not found; install it (apt install libngspice0) or set LABWIRED_NGSPICE_LIB"
    ) from last


def _dispatch_char(msg: bytes, _id: int, _ud) -> int:
    return _ACTIVE._on_char(msg, _id, _ud) if _ACTIVE is not None else 0


def _dispatch_exit(status: int, immediate: bool, quit_: bool, _id: int, _ud) -> int:
    return _ACTIVE._on_exit(status, immediate, quit_, _id, _ud) if _ACTIVE is not None else 0


def _vec_name(probe: str) -> str:
    """Accept `out`, `v(out)`, `V(out)`, `i(vgpio)`; return ngspice's vector name."""
    p = probe.strip()
    if len(p) > 3 and p[0] in "vV" and p[1] == "(" and p[-1] == ")":
        return p[2:-1]
    if len(p) > 3 and p[0] in "iI" and p[1] == "(" and p[-1] == ")":
        return f"{p[2:-1].lower()}#branch"
    return p


_LIB: Optional[ctypes.CDLL] = None
_CALLBACKS: list = []          # keep ctypes callback objects alive for the library's lifetime
_ACTIVE = None                 # the NgSpice or BridgeCircuit that owns the library


def _ensure_lib() -> ctypes.CDLL:
    """Load and initialize libngspice once per process."""
    global _LIB
    if _LIB is None:
        lib = _load_lib()
        lib.ngSpice_Command.argtypes = [ctypes.c_char_p]
        lib.ngSpice_Command.restype = ctypes.c_int
        lib.ngSpice_Circ.argtypes = [ctypes.POINTER(ctypes.c_char_p)]
        lib.ngSpice_Circ.restype = ctypes.c_int
        lib.ngGet_Vec_Info.argtypes = [ctypes.c_char_p]
        lib.ngGet_Vec_Info.restype = ctypes.POINTER(_VectorInfo)
        lib.ngSpice_SetBkpt.argtypes = [ctypes.c_double]
        lib.ngSpice_SetBkpt.restype = ctypes.c_bool
        cb_char = _SEND_CHAR(_dispatch_char)
        cb_stat = _SEND_STAT(lambda *_: 0)
        cb_exit = _CONTROLLED_EXIT(_dispatch_exit)
        _CALLBACKS.extend([cb_char, cb_stat, cb_exit])
        rc = lib.ngSpice_Init(cb_char, cb_stat, cb_exit, None, None, None, None)
        if rc != 0:
            raise NgSpiceError(f"ngSpice_Init failed with {rc}")
        _LIB = lib
    return _LIB


class NgSpice:
    """One SPICE circuit stepped in lockstep.

    libngspice is process-global: creating a new instance unloads the previous
    circuit, so only the newest instance may be stepped.
    """

    def __init__(
        self,
        netlist: str,
        sources: Dict[str, str],
        probes: Dict[str, str],
        log: Optional[Callable[[str], None]] = None,
    ):
        global _ACTIVE
        self._sources = dict(sources)
        self._probes = {k: _vec_name(v) for k, v in probes.items()}
        self._log = log or (lambda s: print(s, file=sys.stderr))
        self._t = 0.0
        self._started = False
        self._pending: Dict[str, float] = {}
        self._fatal: Optional[str] = None

        self._lib = _ensure_lib()
        if _ACTIVE is not None:
            _ACTIVE._detach()
        _ACTIVE = self

        lines = [l for l in netlist.splitlines() if l.strip()]
        if not lines[0].startswith("*"):
            lines.insert(0, "* labwired ngspice co-sim")
        if not any(l.strip().lower() == ".end" for l in lines):
            lines.append(".end")
        arr = (ctypes.c_char_p * (len(lines) + 1))(*[l.encode() for l in lines], None)
        if self._lib.ngSpice_Circ(arr) != 0 or self._fatal:
            raise NgSpiceError(f"ngspice rejected the netlist: {self._fatal or 'see log'}")

    def _detach(self) -> None:
        """Unload this circuit so another instance can load one."""
        try:
            self._lib.ngSpice_Command(b"destroy all")
            self._lib.ngSpice_Command(b"remcirc")
        finally:
            self._started = False

    # -- callbacks
    def _on_char(self, msg: bytes, _id: int, _ud) -> int:
        text = msg.decode(errors="replace")
        if text.startswith("stderr Error") or text.startswith("stderr Fatal"):
            self._fatal = text
        self._log(text)
        return 0

    def _on_exit(self, status: int, immediate: bool, quit_: bool, _id: int, _ud) -> int:
        self._fatal = f"ngspice exit status {status}"
        return 0

    # -- driving
    def _cmd(self, command: str) -> None:
        rc = self._lib.ngSpice_Command(command.encode())
        if rc != 0 or self._fatal:
            raise NgSpiceError(f"ngspice command failed: {command!r}: {self._fatal or rc}")

    def set_source(self, name: str, volts: float) -> None:
        try:
            spice_name = self._sources[name]
        except KeyError:
            raise KeyError(f"unknown source {name!r}; declared: {sorted(self._sources)}") from None
        # Before the transient starts the operating point must come from the
        # netlist's own defaults (a pull-up sits at Vdd, a GPIO at 0); source
        # changes are applied once time is running, like a real pin edge.
        if self._started:
            self._cmd(f"alter {spice_name} = {float(volts):.9g}")
        else:
            self._pending[spice_name] = float(volts)

    def step_to(self, t_end_s: float, max_step_s: Optional[float] = None) -> Dict[str, float]:
        """Advance the transient analysis to `t_end_s` and return the probes."""
        if t_end_s <= self._t:
            return self.read_probes()
        dt = t_end_s - self._t
        tmax = max_step_s or dt / 20.0
        if not self._started:
            # Start the transient and pause at the first timepoint after 0 so
            # the operating point reflects the netlist, then apply the inputs.
            # tstop is a ceiling; breakpoints stop the run early each step.
            tstop = float(os.environ.get("LABWIRED_NGSPICE_TSTOP_S", "1e3"))
            self._cmd("stop when time > 0")
            self._cmd(f"tran {tmax / 2:.12g} {tstop:.12g} 0 {tmax:.12g}")
            self._started = True
            for spice_name, volts in self._pending.items():
                self._cmd(f"alter {spice_name} = {volts:.9g}")
            self._pending.clear()
        self._cmd("delete all")
        self._cmd(f"stop when time >= {t_end_s:.12g}")
        self._cmd("resume")
        self._t = t_end_s
        return self.read_probes()

    def read_probes(self) -> Dict[str, float]:
        out: Dict[str, float] = {}
        for key, vec in self._probes.items():
            info = self._lib.ngGet_Vec_Info(vec.encode())
            if not info or info.contents.v_length == 0 or not info.contents.v_realdata:
                raise NgSpiceError(f"probe {key!r}: vector {vec!r} not found or empty")
            out[key] = float(info.contents.v_realdata[info.contents.v_length - 1])
        return out

    @property
    def time_s(self) -> float:
        return self._t


# --- JSONL server -----------------------------------------------------------

def serve(
    netlist: str,
    sources: Dict[str, str],
    probes: Dict[str, str],
    vdd: float = 3.3,
    stdin: Optional[TextIO] = None,
    stdout: Optional[TextIO] = None,
) -> None:
    """Run the LabWired external_process protocol until stdin closes."""
    stdin = stdin or sys.stdin
    stdout = stdout or sys.stdout
    sim = NgSpice(netlist, sources, probes)
    for line in stdin:
        if not line.strip():
            continue
        msg = json.loads(line)
        for name, value in (msg.get("inputs") or {}).items():
            if name not in sources:
                continue  # routed but not a circuit source: ignore
            if isinstance(value, bool):
                volts = vdd if value else 0.0
            elif isinstance(value, (int, float)):
                volts = float(value)
            else:
                try:
                    volts = float(value)
                except (TypeError, ValueError):
                    raise NgSpiceError(f"input {name!r}: cannot convert {value!r} to volts")
            sim.set_source(name, volts)
        # CosimRunner hands over `time_ns` = the boundary being reached, i.e. the
        # END of the interval [time_ns - dt_ns, time_ns].
        t_end = int(msg.get("time_ns", 0)) / 1e9
        outputs = sim.step_to(t_end)
        stdout.write(json.dumps({"outputs": outputs}) + "\n")
        stdout.flush()


# --- adapter: ngspice host --------------------------------------------------
#
# The native half of the `adapter: ngspice` contract. LabWired core validates
# the manifest (`labwired_core::cosim::ngspice::NgspiceModelSpec`), spawns this
# module with `--bridge-spec <json>`, and exchanges one `external_process`
# JSONL frame per co-simulation boundary. Each step mirrors the browser
# exchange (`createBrowserNgspiceExchange` in the app's ngspice-runtime.ts):
# alter every mapped source to `value * scale + offset`, run the netlist's own
# `.tran` to exactly `time_ns`, check the solver time, and read every output
# node as `raw * scale + offset` (compared with `threshold` for a logic output).

_TIME_TOLERANCE_ULPS = 32


class BridgeCircuit:
    """One `adapter: ngspice` circuit, stepped to exact co-simulation boundaries."""

    def __init__(self, spec: dict, log: Optional[Callable[[str], None]] = None):
        global _ACTIVE
        self.model_id = spec["model_id"]
        self._inputs = spec["inputs"]
        self._outputs = spec["outputs"]
        self._step_ns = spec["step_ns"]
        self._log = log or (lambda _s: None)
        self._diagnostics: list = []
        self._error: Optional[str] = None
        self._started = False
        self._last_time_ns: Optional[int] = None
        self._lib = _ensure_lib()
        if _ACTIVE is not None:
            _ACTIVE._detach()
        _ACTIVE = self
        path = spec["netlist_path"]
        with open(path, encoding="utf-8") as f:
            lines = [line.rstrip("\r") for line in f.read().splitlines() if line.strip()]
        arr = (ctypes.c_char_p * (len(lines) + 1))(*[line.encode() for line in lines], None)
        self._error = None
        # `.include` / `.lib` paths resolve from the netlist's own directory;
        # ngspice reads them while it parses the circuit.
        cwd = os.getcwd()
        os.chdir(os.path.dirname(os.path.abspath(path)))
        try:
            rc = self._lib.ngSpice_Circ(arr)
        finally:
            os.chdir(cwd)
        if rc != 0 or self._error:
            raise NgSpiceError(self._fail("netlist was rejected"))

    def _detach(self) -> None:
        self._lib.ngSpice_Command(b"remcirc")

    def _on_char(self, msg: bytes, _id: int, _ud) -> int:
        text = msg.decode(errors="replace")
        self._diagnostics = (self._diagnostics + [text])[-16:]
        if text.startswith("stderr Error") or text.startswith("stderr Fatal"):
            self._error = self._error or text[len("stderr "):]
        self._log(text)
        return 0

    def _on_exit(self, status: int, _immediate: bool, _quit: bool, _id: int, _ud) -> int:
        self._error = self._error or f"ngspice exited with status {status}"
        return 0

    def _fail(self, what: str) -> str:
        detail = self._error or "; ".join(self._diagnostics[-4:])
        return f"ngspice model '{self.model_id}' {what}" + (f": {detail}" if detail else "")

    def _cmd(self, command: str) -> None:
        self._error = None
        rc = self._lib.ngSpice_Command(command.encode())
        if rc != 0 or self._error:
            raise NgSpiceError(self._fail(f"command {command!r} failed"))

    def _vector_last(self, name: str) -> Optional[float]:
        info = self._lib.ngGet_Vec_Info(name.encode())
        if not info or info.contents.v_length < 1 or not info.contents.v_realdata:
            return None
        return float(info.contents.v_realdata[info.contents.v_length - 1])

    def _vector_first(self, name: str) -> Optional[float]:
        info = self._lib.ngGet_Vec_Info(name.encode())
        if not info or info.contents.v_length < 1 or not info.contents.v_realdata:
            return None
        return float(info.contents.v_realdata[0])

    def _input_volts(self, name: str, source: dict, inputs: dict) -> float:
        raw = inputs.get(name)
        if raw is None:
            # A routed input can be absent before firmware drives it; use the
            # authored default or an inactive low, as the browser does.
            raw = source.get("default_value")
            if raw is None:
                raw = False if source["logic"] else 0.0
        if source["logic"]:
            if not isinstance(raw, bool):
                raise NgSpiceError(f"ngspice model '{self.model_id}' GPIO input '{name}' must be boolean")
            value = 1.0 if raw else 0.0
        elif isinstance(raw, bool):
            value = 1.0 if raw else 0.0
        elif isinstance(raw, (int, float)) and math.isfinite(raw):
            value = float(raw)
        else:
            raise NgSpiceError(f"ngspice model '{self.model_id}' input '{name}' must be a finite number")
        volts = value * source["scale"] + source["offset"]
        if not math.isfinite(volts):
            raise NgSpiceError(f"ngspice model '{self.model_id}' input '{name}' is outside the finite range")
        return volts

    def exchange(self, request: dict) -> Dict[str, object]:
        time_ns, dt_ns = request.get("time_ns"), request.get("dt_ns")
        if not (isinstance(time_ns, int) and not isinstance(time_ns, bool) and time_ns >= 0
                and isinstance(dt_ns, int) and not isinstance(dt_ns, bool) and dt_ns >= 0):
            raise NgSpiceError(f"ngspice model '{self.model_id}' received an invalid simulation time")
        priming = time_ns == 0 and dt_ns == 0 and self._last_time_ns is None
        if not priming and dt_ns < 1:
            raise NgSpiceError(f"ngspice model '{self.model_id}' received an invalid simulation time")
        if self._last_time_ns is not None and time_ns <= self._last_time_ns:
            raise NgSpiceError(f"ngspice model '{self.model_id}' time did not advance monotonically")
        inputs = request.get("inputs")
        if not isinstance(inputs, dict):
            raise NgSpiceError(f"ngspice model '{self.model_id}' received malformed inputs")
        for name, source in self._inputs.items():
            volts = self._input_volts(name, source, inputs)
            self._cmd(f"alter {source['element']} = {volts:.17g}")

        read = self._vector_last
        if priming:
            # The runner's (0, 0) initialization step. Start the netlist's own
            # `.tran` with the initial inputs and report its first stored
            # point, so the next boundary resumes from the state reported
            # here. A separate `op` would not: under `uic` the transient
            # starts from the initial conditions, not the operating point
            # (measured on ngspice-42: `op` 3.3 V, then 0.314 V at 100 us).
            # Without `uic` the first point is t = 0; with `uic` ngspice
            # stores no t = 0 point and the first is its first internal step,
            # which must still fall before the first boundary. The browser
            # exchange has no such step (it rejects dt_ns = 0).
            self._cmd("delete all")
            self._cmd("stop after 1")
            self._cmd("run")
            self._started = True
            first_t = self._vector_first("time")
            step_s = self._step_ns / 1e9
            if first_t is None or not 0.0 <= first_t < step_s:
                raise NgSpiceError(
                    f"ngspice model '{self.model_id}' first transient point is at {first_t} s; "
                    f"it must fall before the first boundary at {step_s} s"
                )
            read = self._vector_first
        else:
            t = time_ns / 1e9
            # A breakpoint makes ngspice place a timepoint exactly at t; the
            # `stop when` condition alone can step past it (measured on
            # ngspice-42: a 200 us stop ran to the .tran end time).
            self._cmd("delete all")
            if not self._lib.ngSpice_SetBkpt(t):
                raise NgSpiceError(self._fail(f"breakpoint at {t!r} s was refused"))
            self._cmd(f"stop when time = {t:.17g}")
            self._cmd("resume" if self._started else "run")
            self._started = True
            solver_t = self._vector_last("time")
            tolerance = sys.float_info.epsilon * max(1.0, abs(t)) * _TIME_TOLERANCE_ULPS
            if solver_t is None or abs(solver_t - t) > tolerance:
                raise NgSpiceError(
                    f"ngspice model '{self.model_id}' stopped at {solver_t} s; requested {t} s "
                    f"(tolerance {tolerance} s)"
                )

        outputs: Dict[str, object] = {}
        for name, output in self._outputs.items():
            node = output["node"]
            raw = 0.0 if node == "0" else read(node)
            if raw is None or not math.isfinite(raw):
                raise NgSpiceError(
                    f"ngspice model '{self.model_id}' output node '{node}' was not found or is not finite"
                )
            scaled = raw * output["scale"] + output["offset"]
            if not math.isfinite(scaled):
                raise NgSpiceError(f"ngspice model '{self.model_id}' output '{name}' is not finite")
            threshold = output.get("threshold")
            outputs[name] = scaled >= threshold if threshold is not None else scaled
        self._last_time_ns = time_ns
        return outputs


def serve_bridge(spec: dict, stdin: Optional[TextIO] = None, stdout: Optional[TextIO] = None) -> int:
    """Serve `adapter: ngspice` frames until stdin closes or a step fails.

    A failure is answered with one `{"error": ...}` frame and ends the host:
    a solver that failed mid-step cannot be resumed as though it had not.
    """
    stdin = stdin or sys.stdin
    stdout = stdout or sys.stdout
    log = (lambda s: print(s, file=sys.stderr)) if os.environ.get("LABWIRED_NGSPICE_LOG") else None
    circuit: Optional[BridgeCircuit] = None
    startup_error: Optional[str] = None
    try:
        circuit = BridgeCircuit(spec, log=log)
    except (NgSpiceError, OSError) as error:
        startup_error = str(error)
    for line in stdin:
        if not line.strip():
            continue
        try:
            if startup_error is not None:
                raise NgSpiceError(startup_error)
            outputs = circuit.exchange(json.loads(line))
        except (NgSpiceError, ValueError, KeyError, TypeError) as error:
            stdout.write(json.dumps({"error": str(error)}) + "\n")
            stdout.flush()
            return 1
        stdout.write(json.dumps({"outputs": outputs}) + "\n")
        stdout.flush()
    return 0


def _main(argv: Iterable[str]) -> int:  # pragma: no cover - thin CLI
    import argparse

    argv = list(argv)
    if argv[:1] == ["--bridge-spec"] and len(argv) == 2:
        return serve_bridge(json.loads(argv[1]))
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("netlist", help="SPICE netlist file")
    ap.add_argument("--source", action="append", default=[], metavar="NAME=VSRC",
                    help="LabWired input name -> SPICE voltage source (repeatable)")
    ap.add_argument("--probe", action="append", default=[], metavar="NAME=v(node)",
                    help="LabWired output name -> node vector (repeatable)")
    ap.add_argument("--vdd", type=float, default=3.3, help="volts for boolean inputs (default 3.3)")
    a = ap.parse_args(list(argv))
    kv = lambda items: dict(s.split("=", 1) for s in items)  # noqa: E731
    with open(a.netlist) as f:
        serve(f.read(), kv(a.source), kv(a.probe), vdd=a.vdd)
    return 0


if __name__ == "__main__":  # pragma: no cover
    sys.exit(_main(sys.argv[1:]))
