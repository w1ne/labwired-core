"""LabWired shared-memory co-simulation client (Linux, standard library only).

Opt in from the manifest with ``config: {transport: shm}``. A model that never
calls :func:`serve` stays a plain JSONL model.

    from labwired_shm import serve

    def step(time_ns, dt_ns, inputs):
        return {"v_out": 3.3 if inputs.get("gpio") else 0.0}

    serve(step)

``inputs`` holds only the signals the host supplied this step; the returned
dict may omit an output to leave it absent. Layout and protocol:
crates/core/src/cosim/shm.rs.
"""
import ctypes
import mmap
import os
import struct
import sys
import time

_MAGIC = 0x4D53574C
_SPIN_NS = 150_000
_SYS_FUTEX = 202  # x86_64; overridden below for other Linux architectures
_FUTEX_WAIT, _FUTEX_WAKE = 0, 1
_libc = ctypes.CDLL(None, use_errno=True)
_libc.syscall.restype = ctypes.c_long
_SYS_FUTEX = {"x86_64": 202, "aarch64": 98, "armv7l": 240, "riscv64": 98}.get(
    os.uname().machine, 202
)


class _Timespec(ctypes.Structure):
    _fields_ = [("tv_sec", ctypes.c_long), ("tv_nsec", ctypes.c_long)]


def _futex_wait(addr, expected, timeout_s):
    ts = _Timespec(int(timeout_s), int((timeout_s % 1) * 1e9))
    _libc.syscall(_SYS_FUTEX, ctypes.c_void_p(addr), _FUTEX_WAIT,
                  ctypes.c_uint32(expected), ctypes.byref(ts), None, 0)


def _futex_wake(addr):
    _libc.syscall(_SYS_FUTEX, ctypes.c_void_p(addr), _FUTEX_WAKE,
                  ctypes.c_int(0x7FFFFFFF), None, None, 0)


def serve(step):
    """Run ``step(time_ns, dt_ns, inputs) -> outputs`` until the host closes."""
    path = os.environ.get("LABWIRED_COSIM_SHM")
    if not path:
        raise RuntimeError("LABWIRED_COSIM_SHM is not set: not started by LabWired shm")
    fd = os.open(path, os.O_RDWR)
    size = os.fstat(fd).st_size
    mem = mmap.mmap(fd, size)
    os.close(fd)
    buf = (ctypes.c_char * size).from_buffer(mem)
    base = ctypes.addressof(buf)
    magic, version, n_in, n_out, names_off, names_len, slots_off = struct.unpack_from(
        "<7I", mem, 0)
    if magic != _MAGIC or version != 1:
        raise RuntimeError("bad LabWired shm header")
    names = bytes(mem[names_off:names_off + names_len]).split(b"\0")[:-1]
    in_names = [n.decode() for n in names[:n_in]]
    out_names = [n.decode() for n in names[n_in:n_in + n_out]]
    unpack_u32 = struct.Struct("<I").unpack_from
    pack_u32 = struct.Struct("<I").pack_into
    slot = struct.Struct("<IIQ")
    f64 = struct.Struct("<d")

    seen = unpack_u32(mem, 64)[0]
    sys.stdout.write('{"labwired_shm":1}\n')
    sys.stdout.flush()
    while True:
        started = time.monotonic_ns()
        while True:
            cur = unpack_u32(mem, 64)[0]
            if cur != seen:
                break
            if time.monotonic_ns() - started < _SPIN_NS:
                continue
            pack_u32(mem, 68, 1)
            if unpack_u32(mem, 64)[0] == seen:
                _futex_wait(base + 64, seen, 0.05)
            pack_u32(mem, 68, 0)
        seen = cur
        if unpack_u32(mem, 136)[0]:
            return
        time_ns, dt_ns = struct.unpack_from("<QQ", mem, 192)
        inputs = {}
        for i, name in enumerate(in_names):
            present, kind, bits = slot.unpack_from(mem, slots_off + 16 * i)
            if not present:
                continue
            if kind == 2:
                inputs[name] = struct.unpack("<d", struct.pack("<Q", bits))[0]
            elif kind == 1:
                inputs[name] = bits - (1 << 64) if bits >> 63 else bits
            else:
                inputs[name] = bool(bits)
        outputs = step(time_ns, dt_ns, inputs)
        for i, name in enumerate(out_names):
            off = slots_off + 16 * (n_in + i)
            value = outputs.get(name)
            if value is None:
                pack_u32(mem, off, 0)
            elif isinstance(value, bool):
                slot.pack_into(mem, off, 1, 0, int(value))
            elif isinstance(value, int):
                slot.pack_into(mem, off, 1, 1, value & 0xFFFFFFFFFFFFFFFF)
            else:
                slot.pack_into(mem, off, 1, 2, struct.unpack("<Q", f64.pack(value))[0])
        pack_u32(mem, 128, seen)
        if unpack_u32(mem, 132)[0]:
            _futex_wake(base + 128)
