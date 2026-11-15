"""Check the Modbus RTU bus frames against an independent decoder.

The decoder and the request encoder are pymodbus's RTU framer, not our code. The
frames come from the simulator:

* the Arduino Uno example (ModbusMaster firmware, MAX485, two XY-MD02 sensors) writes
  every frame that crossed the A/B pair to a JSON file, and each one is decoded
  and its CRC checked here;
* requests built by pymodbus are replayed through a transceiver with two
  slaves, and the answers are decoded the same way. A wrong CRC, a frame split
  by more than 3.5 character times and a request for another address must each
  get no answer.

Run: `python -m pip install pymodbus pytest && python -m pytest scripts/ci/test_modbus_pymodbus.py`
(needs cargo; builds the `rs485_modbus` test in release mode).
"""

import json
import os
import subprocess
from pathlib import Path

import pytest

pymodbus = pytest.importorskip("pymodbus")
from pymodbus.framer import FramerRTU  # noqa: E402
from pymodbus.pdu import DecodePDU  # noqa: E402
from pymodbus.pdu.register_message import (  # noqa: E402
    ReadHoldingRegistersRequest,
    ReadInputRegistersRequest,
    WriteMultipleRegistersRequest,
    WriteSingleRegisterRequest,
)

ROOT = Path(__file__).resolve().parents[2]


def cargo_test(filter_: str, env: dict) -> None:
    subprocess.run(
        ["cargo", "test", "--release", "-p", "labwired-core", "--test", "misc", f"rs485_modbus::{filter_}"],
        cwd=ROOT,
        env={**os.environ, **env},
        check=True,
    )


def decode(hex_text: str, server: bool):
    """(bytes used, device id, pdu) for one frame; used == 0 means pymodbus
    rejected it (bad CRC or short)."""
    decoder = DecodePDU(server)
    used, dev_id, _tid, payload = FramerRTU(decoder).decode(bytes.fromhex(hex_text))
    return used, dev_id, (decoder.decode(payload) if payload else None)


@pytest.fixture(scope="session")
def example_frames(tmp_path_factory):
    out = tmp_path_factory.mktemp("modbus") / "frames.json"
    cargo_test("example", {"LABWIRED_MODBUS_FRAMES_OUT": str(out)})
    return json.loads(out.read_text())


def test_every_frame_on_the_bus_decodes_with_a_valid_crc(example_frames):
    assert len(example_frames) >= 12
    for f in example_frames:
        used, dev_id, pdu = decode(f["hex"], server=(f["from"] == "master"))
        assert used == len(bytes.fromhex(f["hex"])), f"pymodbus rejected {f}"
        assert pdu is not None and dev_id in (1, 2), f


def test_the_master_polls_both_slaves_and_each_answers_for_itself(example_frames):
    polled = {}
    for req, rsp in zip(example_frames, example_frames[1:]):
        if req["from"] != "master" or rsp["from"] == "master":
            continue
        _, req_id, req_pdu = decode(req["hex"], True)
        _, rsp_id, rsp_pdu = decode(rsp["hex"], False)
        assert req_id == rsp_id, "the answer carries the address that was asked"
        assert rsp["from"] == f"s{req_id}"
        assert rsp_pdu.function_code in (req_pdu.function_code, req_pdu.function_code | 0x80)
        if isinstance(req_pdu, ReadInputRegistersRequest):
            assert (req_pdu.address, req_pdu.count) == (1, 2)  # 0x0001 temperature, 0x0002 humidity
            polled.setdefault(req_id, []).append(list(rsp_pdu.registers))
    assert set(polled) == {1, 2}
    assert polled[1][0] == [215, 480]  # 21.5 C, 48.0 %RH
    assert polled[2][0] == [190, 555]  # 19.0 C, 55.5 %RH


def test_the_exception_and_the_write_are_what_the_firmware_asked_for(example_frames):
    decoded = [(f["from"], *decode(f["hex"], f["from"] == "master")) for f in example_frames]
    exceptions = [p for who, _, _, p in decoded if who != "master" and p.function_code & 0x80]
    assert len(exceptions) == 1
    assert (exceptions[0].function_code, exceptions[0].exception_code) == (0x83, 2)
    writes = [(dev, p) for who, _, dev, p in decoded if who == "master" and p.function_code == 6]
    assert len(writes) == 1 and (writes[0][0], writes[0][1].address, writes[0][1].registers) == (2, 0x103, [5])  # temperature correction +0.5 C


def test_slave_1_reports_the_warmer_temperature_after_the_stimulus(example_frames):
    temps = []
    for f in example_frames:
        if f["from"] == "s1":
            _, _, pdu = decode(f["hex"], False)
            if pdu.function_code == 4:
                temps.append(pdu.registers[0])
    assert temps[0] == 215 and 250 in temps


def request(pdu, wait_ms=100):
    frame = FramerRTU(DecodePDU(False)).buildFrame(pdu).hex(" ").upper()
    return {"hex": frame, "wait_ms": wait_ms}


@pytest.fixture(scope="session")
def replay(tmp_path_factory):
    flip = lambda c: {**c, "hex": c["hex"][:-1] + ("0" if c["hex"][-1] != "0" else "1")}
    good = request(ReadInputRegistersRequest(address=1, count=2, dev_id=1))
    full = bytes.fromhex(good["hex"])
    half = lambda b: " ".join(f"{x:02X}" for x in b)
    scenarios = [
        {"name": "read-1", "chunks": [good]},
        {"name": "read-2", "chunks": [request(ReadInputRegistersRequest(address=1, count=2, dev_id=2))]},
        {"name": "holding", "chunks": [request(ReadHoldingRegistersRequest(address=0x101, count=2, dev_id=1))]},
        {"name": "bad-address-register", "chunks": [request(ReadInputRegistersRequest(address=7, count=1, dev_id=1))]},
        {"name": "write-1", "chunks": [request(WriteSingleRegisterRequest(address=0x103, registers=[5], dev_id=1))]},
        {"name": "write-many", "chunks": [request(WriteMultipleRegistersRequest(address=0x103, registers=[0xFFF6], dev_id=1))]},
        {"name": "wrong-crc", "chunks": [flip(good)]},
        {"name": "wrong-address", "chunks": [request(ReadInputRegistersRequest(address=1, count=2, dev_id=9))]},
        # 4 ms of silence between the halves is more than 3.5 characters at 9600 baud.
        {"name": "gap-too-long", "chunks": [
            {"hex": half(full[:4]), "wait_ms": 6}, {"hex": half(full[4:]), "wait_ms": 100}]},
        # 1 ms is inside the gap: still one frame.
        {"name": "gap-short-enough", "chunks": [
            {"hex": half(full[:4]), "wait_ms": 1}, {"hex": half(full[4:]), "wait_ms": 100}]},
    ]
    d = tmp_path_factory.mktemp("replay")
    (d / "in.json").write_text(json.dumps(scenarios))
    cargo_test(
        "replay_requests_from_the_pymodbus_gate",
        {"LABWIRED_MODBUS_REPLAY_IN": str(d / "in.json"), "LABWIRED_MODBUS_REPLAY_OUT": str(d / "out.json")},
    )
    return {r["name"]: r for r in json.loads((d / "out.json").read_text())}


def answer(replay, name):
    rx = replay[name]["rx"]
    return decode(rx, server=False) if rx else None


def test_valid_requests_get_answers_pymodbus_accepts(replay):
    used, dev, pdu = answer(replay, "read-1")
    assert (dev, list(pdu.registers)) == (1, [215, 480]) and used == 9
    _, dev, pdu = answer(replay, "read-2")
    assert (dev, pdu.registers[0]) == (2, 215)
    _, dev, pdu = answer(replay, "holding")
    # Holding registers 0x0101 and 0x0102: address 1 and 9600 baud (0x2580).
    assert (dev, list(pdu.registers)) == (1, [1, 9600])
    _, _, pdu = answer(replay, "write-1")
    assert (pdu.address, pdu.registers) == (0x103, [5])
    _, _, pdu = answer(replay, "write-many")
    assert (pdu.address, pdu.count) == (0x103, 1)
    _, _, pdu = answer(replay, "gap-short-enough")
    assert list(pdu.registers) == [215, 480]


def test_an_unknown_register_is_an_exception_pymodbus_decodes(replay):
    _, dev, pdu = answer(replay, "bad-address-register")
    assert (dev, pdu.function_code, pdu.exception_code) == (1, 0x84, 2)


@pytest.mark.parametrize("name", ["wrong-crc", "wrong-address", "gap-too-long"])
def test_no_answer_for_a_bad_frame(replay, name):
    assert replay[name]["rx"] == "", replay[name]
    assert not any("slave" in line for line in replay[name]["bus"]), replay[name]
